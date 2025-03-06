use color_eyre::eyre::{eyre, Context, ContextCompat, Result};
use color_eyre::Report;
use itertools::Itertools;
use libc::{
    iovec, prlimit64, rlimit64, MCL_CURRENT, MCL_FUTURE, MCL_ONFAULT, RLIMIT_MEMLOCK, RLIM_INFINITY,
};
use memmap2::Mmap;
use once_cell::sync::OnceCell;
use rustix::fd::{AsFd, BorrowedFd};
use rustix::fs::{open, openat, readlinkat, Dir, Mode, OFlags, RawDir};
use rustix::process::{pidfd_getfd, pidfd_open, ForeignRawFd, Pid, PidfdFlags, PidfdGetfdFlags};
use std::io::{IoSlice, IoSliceMut, Read};
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
use syscalls::{syscall, Errno, Sysno};

use crate::{extract_num, DEBUG};

#[derive(Debug, Clone)]
pub struct ProcessHandle {
    pub pid: u32,
    pub parent_pid: u32,
    pub executable: Option<String>,
    pub executable_mmap: Arc<OnceCell<Mmap>>,
    pub proc_dirfd: Arc<OwnedFd>,
    pub pidfd: Arc<OwnedFd>,
    pub fd_mmaps: Arc<OnceCell<Vec<Result<Mmap, std::io::Error>>>>,
}

#[derive(Debug, Clone)]
pub struct ThreadHandle {
    pub pid: u32,
    pub tid: u32,
    pub comm: String,
    pub proc_dirfd: Arc<OwnedFd>,
}

const THREADED_DROP: bool = false;

fn read_string_from_fd(fd: OwnedFd) -> Result<String> {
    let mut file = File::from(fd);
    let mut buf = String::new();
    file.read_to_string(&mut buf)?;
    Ok(buf)
}

fn read_string_from_dirfd(dirfd: BorrowedFd<'_>, path: impl rustix::path::Arg) -> Result<String> {
    let fd = openat(dirfd, path, OFlags::RDONLY, Mode::empty())?;
    read_string_from_fd(fd)
}

/// https://stackoverflow.com/questions/1401359/understanding-linux-proc-pid-maps-or-proc-self-maps
#[derive(Debug, Clone)]
pub struct ProcessMemoryRegion {
    pub address: usize,
    pub len: usize,
    pub can_read: bool,
    pub can_write: bool,
    pub can_exec: bool,
    pub is_private: bool,
    pub offset: usize,
    pub dev: String,
    pub inode: usize,
    pub pathname: Option<String>,
}

impl ProcessMemoryRegion {
    pub fn as_remote_iovec(&self) -> iovec {
        iovec {
            iov_base: self.address as _,
            iov_len: self.len,
        }
    }
}

impl ProcessHandle {
    pub fn parent(&self) -> Result<ProcessHandle> {
        Self::from_pid(self.parent_pid)
    }

    pub fn current() -> Result<ProcessHandle> {
        Self::from_pid(id())
    }

    pub fn thread_ids(&self) -> Result<Vec<u32>> {
        let tasks = openat(&self.proc_dirfd, "task", OFlags::DIRECTORY, Mode::empty())?;
        ls_dir_fd_ints(tasks.as_fd())
    }

    pub fn threads(&self) -> Result<Vec<ThreadHandle>> {
        let tasks = openat(&self.proc_dirfd, "task", OFlags::DIRECTORY, Mode::empty())?;

        let thread_ids = ls_dir_fd_ints(tasks.as_fd())?;
        thread_ids
            .into_iter()
            .map(|tid| {
                let proc_dirfd = openat(
                    tasks.as_fd(),
                    tid.to_string(),
                    OFlags::DIRECTORY,
                    Mode::empty(),
                )?;

                Ok(ThreadHandle {
                    pid: self.pid,
                    tid,
                    comm: read_string_from_dirfd(proc_dirfd.as_fd(), "comm")?
                        .trim()
                        .to_string(),
                    proc_dirfd: Arc::new(proc_dirfd),
                })
            })
            .collect()
    }

    pub fn lock_executable(&self) -> Result<()> {
        self.executable_mmap.get_or_try_init(|| {
            println!(
                "locking executable for {:?} ({})",
                self.executable, self.pid
            );
            let fd = openat(
                self.proc_dirfd.as_fd(),
                "exe",
                OFlags::RDONLY,
                Mode::empty(),
            )?;
            let file = File::from(fd);
            unsafe { Ok(Mmap::map(&file)?) as Result<_> }
        })?;
        Ok(())
    }

    pub fn all_fds(&self) -> Result<Vec<ForeignRawFd>> {
        let fds_dir = openat(&self.proc_dirfd, "fd", OFlags::DIRECTORY, Mode::empty())?;
        Ok(ls_dir_fd_ints(fds_dir.as_fd())?
            .into_iter()
            .map(|x| x as _)
            .collect())
    }

    pub fn lock_fds(&self) -> Result<()> {
        self.fd_mmaps.get_or_try_init(|| {
            println!("locking fds for {:?} ({})", self.executable, self.pid);
            let mut mmaps = vec![];
            for fd in self.all_fds()? {
                let fd = pidfd_getfd(&self.proc_dirfd, fd, PidfdGetfdFlags::empty());
                let Ok(fd) = fd else {
                    dbg!(&fd);
                    continue;
                };
                mmaps.push(unsafe { Mmap::map(&File::from(fd)) })
            }
            Ok(mmaps) as Result<_>
        })?;
        Ok(())
    }

    pub fn gdb_lock_all(&self) -> Result<()> {
        let vm_lck: usize = read_string_from_dirfd(self.proc_dirfd.as_fd(), "status")?
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

    pub fn status(&self) -> Result<ProcStatus> {
        let proc_status = read_string_from_dirfd(self.proc_dirfd.as_fd(), "status")?;
        Ok(ProcStatus(proc_status))
    }

    pub fn oom_score(&self) -> Result<usize> {
        Ok(
            read_string_from_dirfd(self.proc_dirfd.as_fd(), "oom_score")?
                .trim()
                .parse()?,
        )
    }

    pub fn from_pid(pid: u32) -> Result<Self> {
        let dirfd = open(format!("/proc/{pid}"), OFlags::DIRECTORY, Mode::empty())?;
        // Open pidfd between opening dirfd and accessing dirfd in case the pid no longer refers to the same process between opening dirfd and opening pidfd
        let pidfd = Arc::new(pidfd_open(
            Pid::from_raw(pid as _).unwrap(),
            PidfdFlags::empty(),
        )?);
        let proc_status = read_string_from_dirfd(dirfd.as_fd(), "status")?;
        let ppid = proc_status
            .lines()
            .find(|x| x.starts_with("PPid:"))
            .unwrap()
            .trim_start_matches("PPid:\t")
            .parse()?;
        let exe = readlinkat(dirfd.as_fd(), "exe", vec![])?;
        Ok(Self {
            pid,
            parent_pid: ppid,
            // hack: if filename ends with ` (deleted)`, remove the ending
            executable: exe
                .to_str()
                .map(|exe| exe.strip_suffix(" (deleted)").unwrap_or(exe))
                .ok()
                .map(|x| x.to_string()),
            executable_mmap: Arc::new(OnceCell::new()),
            pidfd,
            proc_dirfd: Arc::new(dirfd),
            fd_mmaps: Arc::new(OnceCell::new()),
        })
    }

    pub fn maps(&self) -> Result<Vec<ProcessMemoryRegion>> {
        Ok(read_string_from_dirfd(self.proc_dirfd.as_fd(), "maps")?
            .trim()
            .lines()
            .map(|line| {
                let parts = line.split_whitespace().collect_vec();
                let (addr, perms, offset, dev, inode, pathname) = match &*parts {
                    [addr, perms, offset, dev, inode, pathname, ..] => {
                        (addr, perms, offset, dev, inode, Some(pathname))
                    }
                    [addr, perms, offset, dev, inode] => (addr, perms, offset, dev, inode, None),
                    _ => unreachable!(),
                };
                let (start, end) = addr.split_once('-').unwrap();
                ProcessMemoryRegion {
                    address: usize::from_str_radix(start, 16).unwrap(),
                    len: usize::from_str_radix(end, 16).unwrap()
                        - usize::from_str_radix(start, 16).unwrap(),
                    can_read: perms.contains('r'),
                    can_write: perms.contains('w'),
                    can_exec: perms.contains('x'),
                    is_private: perms.contains('p'),
                    offset: usize::from_str_radix(offset, 16).unwrap(),
                    dev: dev.to_string(),
                    inode: inode.parse().unwrap(),
                    pathname: pathname.map(|x| x.to_string()),
                }
            })
            .collect())
    }

    pub fn read_all_maps(&self, max_chunk_size: usize) -> Result<usize> {
        let maps = self.maps()?.into_iter().filter(|x| x.can_read);
        let iovecs = maps.into_iter().map(|x| unsafe { x.as_remote_iovec() });

        let mut bytes_read = 0;

        let mut buf = vec![0; max_chunk_size];
        for v in iovecs {
            let mut i = 0;
            while i < v.iov_len {
                let remaining = v.iov_len - i;
                let iovec_chunk = iovec {
                    iov_base: unsafe { v.iov_base.byte_add(i) },
                    iov_len: remaining.min(max_chunk_size),
                };
                unsafe {
                    bytes_read += process_vm_readv(
                        self.pid,
                        &mut [IoSliceMut::new(&mut buf)],
                        &[iovec_chunk],
                    )
                    .unwrap_or(0);
                }
                i += iovec_chunk.iov_len;
            }
            assert!(i == v.iov_len);
        }
        Ok(bytes_read)
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
        if DEBUG {
            if let Some(x) = Arc::get_mut(&mut self.executable_mmap) {
                if x.get().is_some() {
                    println!("dropping a process handle");
                    dbg!(&self);
                    let bt = std::backtrace::Backtrace::capture();
                    println!("{bt}");
                }
            }
        }

        if THREADED_DROP {
            drop_mmap_on_thread(&mut self.executable_mmap);
        }
    }
}

#[derive(Debug)]
pub struct ProcStatus(pub String);

impl ProcStatus {
    pub fn extract_num(&self, prefix: &str) -> Option<usize> {
        extract_num(&self.0, prefix)
    }
}

fn ls_dir_fd_ints(fd: BorrowedFd<'_>) -> Result<Vec<u32>> {
    let mut buf = Vec::with_capacity(8192);
    let mut iter = RawDir::new(fd, buf.spare_capacity_mut());
    let mut result = vec![];
    while let Some(entry) = iter.next() {
        let entry = entry?;
        if let Ok(name) = entry.file_name().to_str() {
            if let Ok(num) = name.parse() {
                result.push(num);
            }
        }
    }
    Ok(result)
}

pub fn get_all_pids() -> impl Iterator<Item = u32> {
    ls_dir_ints("/proc").unwrap()
}

fn ls_dir_ints(path: &str) -> Result<impl Iterator<Item = u32>> {
    Ok(read_dir(path)?
        .filter_map(Result::ok)
        .filter_map(|x| x.file_name().to_str().and_then(|x| x.parse::<u32>().ok())))
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

pub fn process_mrelease(pidfd: BorrowedFd, flags: u32) -> Result<usize> {
    unsafe {
        syscall!(Sysno::process_mrelease, pidfd.as_raw_fd(), flags)
            .wrap_err_with(|| format!("pidfd={pidfd:?}"))
    }
}

/// # Safety
/// https://manpages.debian.org/unstable/manpages-dev/process_vm_readv.2.en.html
/// Remote iovecs are in the address space of the target process
pub unsafe fn process_vm_readv(
    pid: u32,
    local_iov: &mut [IoSliceMut<'_>],
    remote_iov: &[iovec],
) -> Result<usize, Errno> {
    syscall!(
        Sysno::process_vm_readv,
        pid,
        local_iov.as_ptr(),
        local_iov.len(),
        remote_iov.as_ptr(),
        remote_iov.len(),
        0
    )
}
