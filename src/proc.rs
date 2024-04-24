use color_eyre::eyre::{eyre, Context, ContextCompat, Result};
use color_eyre::Report;
use libc::{
    iovec, prlimit64, rlimit64, MCL_CURRENT, MCL_FUTURE, MCL_ONFAULT, RLIMIT_MEMLOCK, RLIM_INFINITY,
};
use memmap::Mmap;
use once_cell::sync::OnceCell;
use rustix::fs::{open, openat, Dir, Mode, OFlags};
use rustix::process::{pidfd_getfd, pidfd_open, ForeignRawFd, Pid, PidfdFlags, PidfdGetfdFlags};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::{
    fs::{read_dir, read_link, read_to_string, File},
    path::PathBuf,
    process::id,
    thread,
};
use std::{io, ptr};
use syscalls::{syscall, Sysno};

#[derive(Debug, Clone)]
pub struct ProcessHandle {
    pub pid: u32,
    pub parent_pid: u32,
    pub executable: Option<PathBuf>,
    pub executable_mmap: Arc<OnceCell<Mmap>>,
    pub proc_dirfd: Arc<OwnedFd>,
    pub pidfd: Arc<OwnedFd>,
    pub fd_mmap: Arc<OnceCell<Vec<Result<Mmap, std::io::Error>>>>,
}

const THREADED_DROP: bool = false;

impl ProcessHandle {
    pub fn parent(&self) -> Result<ProcessHandle> {
        get_info_for_pid(self.parent_pid)
    }

    pub fn current() -> Result<ProcessHandle> {
        get_info_for_pid(id())
    }

    pub fn thread_ids(&self) -> Result<impl Iterator<Item = u32>> {
        ls_dir_ids(&format!("/proc/{}/task", self.pid))
    }

    pub fn lock_executable(&self) -> Result<()> {
        self.executable_mmap.get_or_try_init(|| {
            println!(
                "locking executable for {:?} ({})",
                self.executable, self.pid
            );
            unsafe { Ok(Mmap::map(&File::open(format!("/proc/{}/exe", self.pid))?)?) as Result<_> }
        })?;
        Ok(())
    }

    pub fn all_fds(&self) -> Result<Vec<ForeignRawFd>, std::io::Error> {
        let fds_dir = openat(&self.proc_dirfd, "fd", OFlags::DIRECTORY, Mode::empty())?;
        let dir = Dir::new(fds_dir)?;
        Ok(dir
            .flat_map(|x| {
                x.ok()
                    .and_then(|x| x.file_name().to_str().ok().and_then(|x| x.parse().ok()))
            })
            .collect())
    }

    pub fn lock_fds(&self) -> Result<()> {
        self.fd_mmap.get_or_try_init(|| {
            println!("locking fds for {:?} ({})", self.executable, self.pid);
            let mut mmaps = vec![];
            for fd in self.all_fds()? {
                let fd = pidfd_getfd(&self.pidfd, fd, PidfdGetfdFlags::empty());
                let Ok(fd) = fd else {
                    dbg!(&fd);
                    continue;
                };
                mmaps.push(unsafe { Mmap::map(&File::from(fd.try_clone()?)) })
            }
            Ok(mmaps) as Result<_, std::io::Error>
        })?;
        Ok(())
    }

    pub fn maybe_lock_all(&self) -> Result<()> {
        let vm_lck: usize = read_to_string(format!("/proc/{}/status", self.pid))?
            .lines()
            .find_map(|x| x.strip_prefix("VmLck:"))
            .wrap_err("no VmLck")?
            .strip_suffix("kB")
            .wrap_err("no kB")?
            .trim()
            .parse()?;

        let rlimit = rlimit64 {
            rlim_cur: RLIM_INFINITY,
            rlim_max: RLIM_INFINITY,
        };

        unsafe {
            if prlimit64(self.pid as i32, RLIMIT_MEMLOCK, &rlimit, ptr::null_mut()) != 0 {
                return Err(Report::from(io::Error::last_os_error()));
            }
        }

        let mlock_flags = MCL_CURRENT | MCL_FUTURE | MCL_ONFAULT;

        if vm_lck == 0 {
            let success = Command::new("gdb")
                .stdout(Stdio::inherit())
                .arg("--pid")
                .arg(self.pid.to_string())
                .arg("-ex")
                .arg(format!("call (int) mlockall({mlock_flags})"))
                .arg("-ex")
                .arg("detach")
                .arg("-ex")
                .arg("quit")
                .status()?
                .success();

            if !success {
                return Err(eyre!("gdb failed"));
            }

            println!("locked memory for {:?}", self.executable);
        }

        Ok(())
    }
}

fn drop_mmap_on_thread<T: Send + 'static>(x: &mut Arc<OnceCell<T>>) {
    if let Some(x) = Arc::get_mut(x) {
        if let Some(x) = x.take() {
            thread::spawn(move || {
                drop(x);
            });
        }
    }
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        if let Some(x) = Arc::get_mut(&mut self.executable_mmap) {
            if x.get().is_some() {
                println!("dropping a process handle");
                dbg!(&self);
                let bt = std::backtrace::Backtrace::capture();
                println!("{bt}");
            }
        }

        if THREADED_DROP {
            drop_mmap_on_thread(&mut self.executable_mmap);
        }
    }
}

pub fn get_all_pids() -> impl Iterator<Item = u32> {
    ls_dir_ids("/proc").unwrap()
}

fn ls_dir_ids(path: &str) -> Result<impl Iterator<Item = u32>> {
    Ok(read_dir(path)?
        .filter_map(Result::ok)
        .filter_map(|x| x.file_name().to_str().and_then(|x| x.parse::<u32>().ok())))
}

pub fn get_info_for_pid(pid: u32) -> Result<ProcessHandle> {
    let dirfd = open(format!("/proc/{pid}"), OFlags::DIRECTORY, Mode::empty())?;
    let proc_status = read_to_string(format!("/proc/{pid}/status"))?;
    let ppid = proc_status
        .lines()
        .find(|x| x.starts_with("PPid:"))
        .unwrap()
        .trim_start_matches("PPid:\t")
        .parse()?;
    // dirty hack: if filename ends with ` (deleted)`, remove the ending
    let mut exe = read_link(format!("/proc/{pid}/exe"))?;
    if let Some(exe_str) = exe.as_os_str().to_str() {
        if exe_str.ends_with(" (deleted)") {
            exe = PathBuf::from(&exe_str[0..(exe_str.len() - " (deleted)".len())]);
        }
    }
    Ok(ProcessHandle {
        pid,
        parent_pid: ppid,
        executable: Some(exe),
        executable_mmap: Arc::new(OnceCell::new()),
        proc_dirfd: Arc::new(dirfd),
        pidfd: Arc::new(pidfd_open(
            Pid::from_raw(pid as _).unwrap(),
            PidfdFlags::empty(),
        )?),
        fd_mmap: Arc::new(OnceCell::new()),
    })
}

/// # Safety
/// see https://man7.org/linux/man-pages/man2/process_madvise.2.html
pub unsafe fn process_madvise(
    pidfd: &OwnedFd,
    iovecs: &[iovec],
    advice: i32,
    flags: u32,
) -> Result<usize> {
    unsafe {
        syscall!(
            Sysno::process_madvise,
            pidfd.as_raw_fd(),
            iovecs.as_ptr(),
            iovecs.len(),
            advice,
            flags
        )
        .wrap_err_with(|| format!("pidfd={pidfd:?}"))
    }
}

pub fn process_mrelease(pidfd: &OwnedFd, flags: u32) -> Result<usize> {
    unsafe {
        syscall!(Sysno::process_mrelease, pidfd.as_raw_fd(), flags)
            .wrap_err_with(|| format!("pidfd={pidfd:?}"))
    }
}
