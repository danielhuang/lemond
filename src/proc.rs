use color_eyre::eyre::{ContextCompat, Result};
use color_eyre::Report;
use libc::{prlimit64, rlimit64, RLIMIT_MEMLOCK};
use memmap::Mmap;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::{
    fs::{read_dir, read_link, read_to_string, File},
    path::PathBuf,
    process::id,
    sync::Arc,
    thread,
};
use std::{io, ptr};

#[derive(Debug, Clone)]
pub struct ProcessHandle {
    pub pid: u32,
    pub parent_pid: u32,
    pub executable: Option<PathBuf>,
    pub executable_mmap: Arc<OnceLock<Mmap>>,
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

    pub fn maybe_lock_all(&self) -> Result<()> {
        let vm_lck: usize = read_to_string(format!("/proc/{}/status", self.pid))?
            .lines()
            .find_map(|x| x.strip_prefix("VmLck:"))
            .wrap_err("no VmLck")?
            .strip_suffix("kB")
            .wrap_err("no kB")?
            .trim()
            .parse()?;

        // 1 TB
        let rlimit = rlimit64 {
            rlim_cur: 1024 * 1024 * 1024 * 1024,
            rlim_max: 1024 * 1024 * 1024 * 1024,
        };

        unsafe {
            if prlimit64(self.pid as i32, RLIMIT_MEMLOCK, &rlimit, ptr::null_mut()) != 0 {
                return Err(Report::from(io::Error::last_os_error()));
            }
        }

        if vm_lck == 0 {
            Command::new("gdb")
                .stdout(Stdio::inherit())
                .arg("--pid")
                .arg(self.pid.to_string())
                .arg("-ex")
                .arg("call (int) mlockall(3)")
                .arg("-ex")
                .arg("detach")
                .arg("-ex")
                .arg("quit")
                .status()?
                .exit_ok()?;

            println!("locked memory for {:?}", self.executable);
        }

        Ok(())
    }
}

fn drop_mmap_on_thread<T: Send + 'static>(x: &mut Arc<OnceLock<T>>) {
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
        executable_mmap: Arc::new(OnceLock::new()),
    })
}
