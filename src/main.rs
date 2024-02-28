use cancellable_timer::Timer;
use color_eyre::eyre::{Context, Result};
use color_eyre::{Help, Report};
use lemond::proc::{get_all_pids, get_info_for_pid, pidfd_open, process_mrelease};
use lemond::procmon::ProcMon;
use lemond::{proc::ProcessHandle, Message};
use libc::{
    kill, mlockall, mmap, pid_t, prlimit, prlimit64, rlimit, rlimit64, setpriority, waitpid,
    MCL_CURRENT, MCL_FUTURE, RLIMIT_CPU, RLIMIT_NICE, RLIMIT_RTPRIO, RLIMIT_RTTIME, RLIM_INFINITY,
};
use signal_hook::consts::signal::*;
use signal_hook::iterator::Signals;
use std::collections::HashSet;
use std::env::set_var;
use std::fs::{self, read_to_string, remove_file, File};
use std::io::{self, Read, Write};
use std::mem::size_of;
use std::os::unix::fs::PermissionsExt;
use std::process::exit;
use std::process::id;
use std::ptr::null_mut;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use std::{collections::HashMap, fmt::Debug};
use std::{
    fs::{set_permissions, Permissions},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};
use std::{io::Error, os::unix::net::UnixDatagram};
use syscalls::{syscall, Sysno};

const CRITICAL_PROCESSES: &[&str] = &[
    "/usr/bin/pipewire",
    "/usr/bin/pipewire-pulse",
    "/usr/bin/pipewire-media-session",
    "/usr/bin/wireplumber",
    "/usr/lib/systemd/systemd",
    "/usr/bin/pulseaudio",
    "/usr/bin/pulseeffects",
    "/usr/bin/easyeffects",
    "/usr/bin/kwin_wayland",
    "/usr/bin/gnome-shell",
    "/usr/bin/Xwayland",
    "/usr/lib/Xorg",
    "/usr/bin/gnome-system-monitor",
    "/usr/bin/i3lock",
    "/usr/lib/systemd/systemd-oomd",
    "/usr/bin/ksysguard",
    "/usr/lib/ksysguard/ksgrd_network_helper",
    "/usr/bin/ksysguardd",
    "/usr/bin/Hyprland",
];

const EXCLUDE_REALTIME: &[&str] = &["/usr/bin/gnome-shell"];

const GDB_MLOCK_PROCESSES: &[&str] = &[
    "/usr/lib/Xorg",
    "/usr/bin/gnome-system-monitor",
    "/usr/lib/systemd/systemd-oomd",
    "/usr/bin/pipewire",
    "/usr/bin/pipewire-pulse",
    "/usr/bin/pipewire-media-session",
    "/usr/bin/wireplumber",
    "/usr/bin/easyeffects",
    "/usr/bin/gnome-shell",
];

const LOW_PRIORITY_PROCESSES: &[&str] = &["rustc", "cc", "c++", "gcc", "g++", "makepkg", "cc1"];

const SCHED_IDLEPRIO: i32 = 5;
const SCHED_DEADLINE: i32 = 6;

const SOCKET_PATH: &str = "/run/lemond.socket";

const REALTIME_NICE: i32 = -15;
const BOOST_NICE: i32 = -7;
const LOW_PRIORITY_NICE: i32 = 19;

const ENABLE_REALTIME: bool = true;

const USE_THREAD_IDS: bool = true;

const FORCE_ASSIGN_NORMAL_SCHEDULER: bool = true;

const ENABLE_NICE: bool = true;

const ENABLE_SOCKET: bool = true;
const ENABLE_MLOCK: bool = true;
const ENABLE_GDB_MLOCK: bool = false;

const ENABLE_KERNEL_TWEAKS: bool = true;

const LEMOND_SELF_REALTIME: bool = false;

type ProcessCache = HashMap<u32, Option<ProcessHandle>>;

fn parents(x: &ProcessHandle, process_cache: &ProcessCache) -> Vec<ProcessHandle> {
    let mut result = Vec::new();
    let mut cur = x.clone();

    while let Some(Some(info)) = process_cache.get(&cur.parent_pid) {
        result.push(info.clone());
        cur = info.clone();
    }

    result
}

const SCHED_FLAG_RESET_ON_FORK: u64 = 0x01;
const SCHED_FLAG_RECLAIM: u64 = 0x02;

// from kernel
const SCHED_FIXEDPOINT_SHIFT: u32 = 10;
const SCHED_CAPACITY_SHIFT: u32 = SCHED_FIXEDPOINT_SHIFT;
const SCHED_CAPACITY_SCALE: u32 = 1 << SCHED_CAPACITY_SHIFT;

#[repr(C)]
struct sched_attr {
    size: u32,

    sched_policy: u32,
    sched_flags: u64,

    // SCHED_NORMAL, SCHED_BATCH
    sched_nice: i32,

    // SCHED_FIFO, SCHED_RR
    sched_priority: u32,

    // SCHED_DEADLINE (nsec)
    sched_runtime: u64,
    sched_deadline: u64,
    sched_period: u64,

    // Utilization hints
    sched_util_min: u32,
    sched_util_max: u32,
}

fn set_nice(pid: u32, nice: i32) -> Result<()> {
    handle_libc_errno(unsafe { setpriority(libc::PRIO_PROCESS, pid as _, nice) })
}

pub fn handle_libc_errno(result: i32) -> std::result::Result<(), Report> {
    match result {
        0 => Ok(()),
        -1 => Err(Report::from(Error::last_os_error())),
        _ => unreachable!(),
    }
}

fn set_scheduler(pid: u32, policy: i32, nice: i32, prio: i32, reset_on_fork: bool) -> Result<()> {
    const NS_PER_MS: u64 = 1000 * 1000;

    if ENABLE_NICE {
        set_nice(pid, nice).with_context(|| format!("pid={pid}"))?;
    }

    let attr = sched_attr {
        size: size_of::<sched_attr>() as u32,
        sched_policy: policy as _,
        sched_flags: if reset_on_fork {
            SCHED_FLAG_RESET_ON_FORK | SCHED_FLAG_RECLAIM
        } else {
            SCHED_FLAG_RECLAIM
        },
        sched_nice: nice,
        sched_priority: prio as _,
        sched_runtime: NS_PER_MS * 2,
        sched_deadline: NS_PER_MS * 2,
        sched_period: NS_PER_MS * 2,
        sched_util_min: 0,
        sched_util_max: SCHED_CAPACITY_SCALE - 1,
    };

    let result = unsafe { syscall!(Sysno::sched_setattr, pid, &attr as *const _, 0) };

    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            let e = e.into_raw();
            Err(Report::from(Error::from_raw_os_error(e))
                .with_note(|| format!("pid={pid} policy={policy} nice={nice} prio={prio}")))
        }
    }
}

fn set_realtime(p: &ProcessHandle, pid: u32) -> Result<()> {
    let i = p
        .executable
        .as_ref()
        .and_then(|x| x.as_os_str().to_str())
        .and_then(|x| CRITICAL_PROCESSES.iter().position(|&s| x == s))
        .unwrap_or(10);

    let is_lemond = id() == p.pid;

    if ENABLE_REALTIME
        && p.executable
            .as_ref()
            .and_then(|x| x.as_os_str().to_str())
            .is_some_and(|x| !EXCLUDE_REALTIME.contains(&x))
    {
        let limit = rlimit64 {
            rlim_cur: RLIM_INFINITY,
            rlim_max: RLIM_INFINITY,
        };

        // workaround https://gitlab.freedesktop.org/drm/amd/-/issues/2861
        for rlimit in [RLIMIT_RTTIME, RLIMIT_CPU, RLIMIT_RTPRIO, RLIMIT_NICE] {
            handle_error(unsafe {
                syscall!(
                    Sysno::prlimit64,
                    pid as pid_t,
                    rlimit,
                    &limit as *const rlimit64,
                    0
                )
                .wrap_err_with(|| format!("rlimit={rlimit}"))
            });
        }

        set_scheduler(
            pid,
            libc::SCHED_FIFO,
            REALTIME_NICE,
            99 - i as i32,
            !(is_lemond && LEMOND_SELF_REALTIME),
        )?;
    } else {
        set_scheduler(
            pid,
            libc::SCHED_OTHER,
            REALTIME_NICE,
            0,
            !(is_lemond && LEMOND_SELF_REALTIME),
        )?;
    }
    Ok(())
}

fn set_boosted(_: &ProcessHandle, pid: u32) -> Result<()> {
    set_scheduler(pid, libc::SCHED_OTHER, BOOST_NICE, 0, false)?;
    Ok(())
}

fn set_normal(_: &ProcessHandle, pid: u32) -> Result<()> {
    set_scheduler(pid, libc::SCHED_OTHER, 0, 0, false)?;
    Ok(())
}

fn set_low_priority(_: &ProcessHandle, pid: u32) -> Result<()> {
    set_scheduler(pid, SCHED_IDLEPRIO, LOW_PRIORITY_NICE, 0, false)?;
    Ok(())
}

fn set_all(p: &ProcessHandle, f: fn(&ProcessHandle, u32) -> Result<()>) -> Result<()> {
    f(p, p.pid)?;
    if USE_THREAD_IDS {
        if let Ok(thread_ids) = p.thread_ids() {
            for tid in thread_ids {
                f(p, tid)?;
            }
        }
    }
    Ok(())
}

fn try_set_all(p: &ProcessHandle, f: fn(&ProcessHandle, u32) -> Result<()>) {
    let e = set_all(p, f);
    #[cfg(debug_assertions)]
    handle_error(e);
}

fn is_child_of(
    x: &ProcessHandle,
    mut predicate: impl FnMut(&ProcessHandle) -> bool,
    process_cache: &ProcessCache,
) -> bool {
    let parents = parents(x, process_cache);

    for parent in parents {
        if predicate(&parent) {
            return true;
        }
    }

    false
}

fn is_equal_or_child_of(
    x: &ProcessHandle,
    mut predicate: impl FnMut(&ProcessHandle) -> bool,
    process_cache: &ProcessCache,
) -> bool {
    predicate(x) || is_child_of(x, predicate, process_cache)
}

fn should_be_realtime(p: &ProcessHandle, state: Option<&State>) -> bool {
    if let Some(pid) = state.and_then(|x| x.client_pid) {
        if p.pid == pid {
            return LEMOND_SELF_REALTIME;
        }
    }
    if p.pid == std::process::id() {
        return LEMOND_SELF_REALTIME;
    }
    let exe = &p.executable;
    if let Some(exe) = exe {
        CRITICAL_PROCESSES.iter().any(|&x| exe.to_str() == Some(x))
    } else {
        false
    }
}

#[track_caller]
fn handle_error<T>(r: Result<T>) -> Option<T> {
    if let Err(e) = &r {
        println!("{e:?}");
    }
    r.ok()
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct State {
    foreground_pid: Option<u32>,
    client_pid: Option<u32>,
}

fn socket_handler(
    state: &Mutex<State>,
    running: &AtomicBool,
    process_cache: &Mutex<HashMap<u32, Option<ProcessHandle>>>,
) {
    let _ = remove_file(SOCKET_PATH);
    let socket = UnixDatagram::bind(SOCKET_PATH).unwrap();
    set_permissions(SOCKET_PATH, Permissions::from_mode(0o666)).unwrap();

    while running.load(Ordering::Acquire) {
        let mut buf = vec![0; 65536];
        let len = socket.recv(&mut buf).unwrap();
        let data = &buf[0..len];

        handle_error(process_message(data, state, process_cache));
    }
}

fn process_message(
    buf: &[u8],
    state: &Mutex<State>,
    process_cache: &Mutex<HashMap<u32, Option<ProcessHandle>>>,
) -> Result<bool> {
    let message: Message = serde_json::from_slice(buf)?;
    let mut state = state.lock().unwrap();
    let prev_state = state.clone();
    match message {
        Message::SetForegroundPid { pid } => {
            let prev_pid = state.foreground_pid;

            state.foreground_pid = pid;

            if pid != prev_pid {
                if let Some(pid) = pid {
                    let process = get_info_for_pid(pid)?;
                    update_single_process(&process, &state, &process_cache.lock().unwrap());
                }

                if let Some(prev_pid) = prev_pid {
                    let prev_process = get_info_for_pid(prev_pid)?;
                    update_single_process(&prev_process, &state, &process_cache.lock().unwrap());
                }
            }
        }
        Message::SetClientPid { pid } => {
            state.client_pid = Some(pid);
        }
    }
    Ok(*state != prev_state)
}

fn cleanup() -> Result<()> {
    let all_processes: Vec<_> = get_all_pids().map(get_info_for_pid).collect();
    let process_cache = all_processes
        .iter()
        .filter_map(|x| x.as_ref().ok())
        .map(|x| (x.pid, Some(x.clone())))
        .collect();
    for process in all_processes.into_iter().flatten() {
        if process.executable.is_some()
            && is_equal_or_child_of(
                &process,
                |parent| should_be_realtime(parent, None),
                &process_cache,
            )
        {
            set_all(&process, set_normal)?;
        }
    }
    Ok(())
}

fn total_memory_kb() -> usize {
    let info = read_to_string("/proc/meminfo").unwrap();
    let line = info.lines().find(|x| x.starts_with("MemTotal:")).unwrap();
    let kb = line.split_whitespace().nth(1).unwrap();
    kb.parse().unwrap()
}

fn available_memory_kb() -> usize {
    let info = read_to_string("/proc/meminfo").unwrap();
    let line = info
        .lines()
        .find(|x| x.starts_with("MemAvailable:"))
        .unwrap();
    let kb = line.split_whitespace().nth(1).unwrap();
    kb.parse().unwrap()
}

fn total_memory() -> usize {
    total_memory_kb() * 1024
}

fn kernel_tweaks(revert: Option<Vec<String>>) -> Result<Vec<String>> {
    let values = vec![
        // ("/proc/sys/vm/compaction_proactiveness", "0"),
        ("/proc/sys/vm/min_free_kbytes", "1048576".to_string()),
        ("/proc/sys/vm/swappiness", "15".to_string()),
        ("/sys/kernel/mm/lru_gen/enabled", "5".to_string()),
        ("/sys/kernel/mm/lru_gen/min_ttl_ms", "1000".to_string()),
        ("/proc/sys/vm/zone_reclaim_mode", "0".to_string()),
        // ("/sys/kernel/mm/transparent_hugepage/enabled", "never".to_string()),
        // ("/sys/kernel/mm/transparent_hugepage/shmem_enabled", "never".to_string()),
        // ("/sys/kernel/mm/transparent_hugepage/khugepaged/defrag", "0".to_string()),
        ("/proc/sys/vm/page_lock_unfairness", "1".to_string()),
        ("/proc/sys/kernel/sched_child_runs_first", "0".to_string()),
        // ("/proc/sys/kernel/sched_autogroup_enabled", "0".to_string()),
        (
            "/proc/sys/kernel/sched_cfs_bandwidth_slice_us",
            "500".to_string(),
        ),
        (
            "/sys/kernel/debug/sched/migration_cost_ns",
            "500000".to_string(),
        ),
        ("/sys/kernel/debug/sched/nr_migrate", "8".to_string()),
        // ("/sys/power/image_size", "0".to_string()),
        // ("/sys/power/image_size", total_memory().to_string()),
    ];

    if let Some(revert) = revert {
        assert!(revert.len() == values.len());

        for ((path, _), mut value) in values.into_iter().zip(revert) {
            if !value.is_empty() {
                if value.contains('[') {
                    value = value
                        .chars()
                        .skip_while(|&x| x != '[')
                        .skip(1)
                        .take_while(|&x| x != ']')
                        .collect();
                }
                println!("reverting {path} back to {}", value.trim());
                let mut file = File::create(path)?;
                write!(&mut file, "{value}")?;
            }
        }

        Ok(vec![])
    } else {
        let mut prev = vec![];

        for (path, value) in values {
            if let Ok(mut file) = File::create(path) {
                println!("setting {path} to {value}");
                let prev_value = read_to_string(path)?;
                prev.push(prev_value);
                write!(&mut file, "{value}")?;
            } else {
                println!("{path} does not exist, skipping");
                prev.push("".into());
            }
        }

        Ok(prev)
    }
}

fn main() {
    set_var("RUST_BACKTRACE", "1");

    color_eyre::install().unwrap();

    let running = Arc::new(AtomicBool::new(true));
    let (mut timer, canceller) = Timer::new2().unwrap();
    let mut signals = Signals::new([SIGINT, SIGTERM]).unwrap();
    let state = Mutex::new(State::default());
    let process_cache = Mutex::new(HashMap::new());

    assert!(CRITICAL_PROCESSES
        .iter()
        .collect::<HashSet<_>>()
        .is_superset(&GDB_MLOCK_PROCESSES.iter().collect::<HashSet<_>>()));

    assert!(CRITICAL_PROCESSES
        .iter()
        .collect::<HashSet<_>>()
        .is_superset(&EXCLUDE_REALTIME.iter().collect::<HashSet<_>>()));

    unsafe {
        mlockall(MCL_CURRENT | MCL_FUTURE);
    }

    let cfs_tweaks_prev = if ENABLE_KERNEL_TWEAKS {
        handle_error(kernel_tweaks(None))
    } else {
        None
    };

    println!("started");

    thread::scope(|scope| {
        scope.spawn({
            || {
                signals.forever().next();
                println!("exiting, setting processes back to normal");

                running.store(false, Ordering::Release);
                canceller.cancel().unwrap();
                drop(state.lock().unwrap());

                handle_error(cleanup());

                {
                    let mut process_cache = process_cache.lock().unwrap();
                    process_cache.clear();
                }

                if let Some(cfs_tweaks_prev) = cfs_tweaks_prev {
                    handle_error(kernel_tweaks(Some(cfs_tweaks_prev)));
                }

                exit(0);
            }
        });
        scope.spawn(|| {
            if ENABLE_SOCKET {
                socket_handler(&state, &running, &process_cache);
            }
        });
        scope.spawn(|| {
            let mon = ProcMon::new();

            {
                populate_process_cache(&process_cache);
            }

            while running.load(Ordering::Acquire) {
                let event = mon.wait_for_event();

                let state = state.lock().unwrap();

                let mut process_cache = process_cache.lock().unwrap();

                match event.event_type {
                    Some(lemond::procmon::EventType::Exec) => {
                        if event.pid == event.tgid {
                            let new_pid = event.pid;
                            let process_info = get_info_for_pid(new_pid).ok();
                            process_cache.insert(new_pid, process_info.clone());
                            if let Some(process) = process_info {
                                update_single_process(&process, &state, &process_cache);
                            }
                        }
                    }
                    Some(lemond::procmon::EventType::Exit) => {
                        let old_pid = event.pid;
                        process_cache.remove(&old_pid);
                    }
                    _ => {}
                }
            }
        });
        scope.spawn(|| {
            while running.load(Ordering::Acquire) {
                populate_process_cache(&process_cache);
                update_all_processes_once(&state.lock().unwrap(), &process_cache.lock().unwrap());
                let _ = timer.sleep(Duration::from_millis(5000));
            }
        });
        scope.spawn(|| {
            use std::fmt::Write;
            let mut mem_buf = String::with_capacity(16 * 1024);
            let mut path_buf = String::with_capacity(4096);
            let mut file_buf = String::with_capacity(32 * 1024);

            let total_memory_kb = total_memory_kb();

            while running.load(Ordering::Relaxed) {
                let free_memory_kb = {
                    mem_buf.clear();
                    File::open("/proc/meminfo")
                        .unwrap()
                        .read_to_string(&mut mem_buf)
                        .unwrap();
                    let line = mem_buf
                        .lines()
                        .find(|x| x.starts_with("MemAvailable:"))
                        .unwrap();
                    let kb = line.split_whitespace().nth(1).unwrap();
                    kb.parse::<u64>().unwrap()
                };
                // 5% left or 1GB left
                if free_memory_kb < (total_memory_kb as u64 / 20).max(1024 * 1024) {
                    let start = Instant::now();
                    println!("low on memory! performing oom kill");
                    let cache = process_cache.lock().unwrap();
                    let oom_target = cache
                        .iter()
                        .filter_map(|(pid, process)| {
                            if let Some(process) = process {
                                if !should_be_realtime(process, None) {
                                    return Some((pid, process));
                                }
                            }
                            None
                        })
                        .max_by_key(|(pid, _)| {
                            path_buf.clear();
                            write!(&mut path_buf, "/proc/{}/status", pid).unwrap();
                            if let Ok(mut file) = File::open(&path_buf) {
                                file_buf.clear();
                                file.read_to_string(&mut file_buf).unwrap();
                                let size = file_buf
                                    .lines()
                                    .find_map(|x| x.strip_prefix("VmData:"))
                                    .unwrap()
                                    .strip_suffix("kB")
                                    .unwrap()
                                    .trim()
                                    .parse::<usize>()
                                    .unwrap();
                                Some(size)
                            } else {
                                None
                            }
                        })
                        .unwrap();
                    let pid = *oom_target.0;
                    println!(
                        "killing target (pid={}, exe={:?}) took {:?} to find",
                        pid,
                        oom_target.1.executable,
                        start.elapsed(),
                    );
                    drop(cache);
                    let kill_start = Instant::now();
                    handle_error(pidfd_open(pid as i32).and_then(|x| process_mrelease(&x, 0)));
                    unsafe {
                        loop {
                            if kill(pid as i32, SIGKILL) != 0 {
                                println!("process killed in {:?}", start.elapsed());
                                break;
                            }
                            thread::sleep(Duration::from_millis(25));
                            if kill_start.elapsed().as_secs_f64() > 1.0 {
                                println!("kill is taking too long! bailing");
                                break;
                            }
                        }
                    }
                    continue;
                } else {
                    fs::write("/sys/power/image_size", (free_memory_kb * 1024).to_string())
                        .unwrap();
                }
                thread::sleep(Duration::from_millis((free_memory_kb / 4096).min(1000)));
            }
        });
    });
}

fn populate_process_cache(process_cache: &Mutex<HashMap<u32, Option<ProcessHandle>>>) {
    let mut process_cache = process_cache.lock().unwrap();
    let pids: HashSet<_> = get_all_pids().collect();
    for &pid in &pids {
        process_cache
            .entry(pid)
            .or_insert_with(|| get_info_for_pid(pid).ok());
    }
    process_cache.retain(|pid, handle| pids.contains(pid) && handle.is_some());
}

fn update_all_processes_once(state: &State, process_cache: &ProcessCache) {
    for process in process_cache.values().flatten() {
        update_single_process(process, state, process_cache);
    }
}

fn update_single_process(
    process: &ProcessHandle,
    state: &State,
    process_cache: &HashMap<u32, Option<ProcessHandle>>,
) {
    if process.executable.is_some() {
        if should_be_realtime(process, Some(state)) {
            try_set_all(process, set_realtime);
            if ENABLE_MLOCK {
                handle_error(process.lock_executable());
            }
            if ENABLE_GDB_MLOCK
                && GDB_MLOCK_PROCESSES
                    .iter()
                    .any(|&x| process.executable.as_ref().and_then(|x| x.to_str()) == Some(x))
            {
                handle_error(process.maybe_lock_all());
            }
        } else if is_child_of(
            process,
            |parent| should_be_realtime(parent, Some(state)),
            process_cache,
        ) {
            let should_boost = is_equal_or_child_of(
                process,
                |parent| Some(parent.pid as _) == state.foreground_pid,
                process_cache,
            );
            if should_boost {
                try_set_all(process, set_boosted);
            } else if LOW_PRIORITY_PROCESSES.iter().any(|&x| {
                process
                    .executable
                    .as_ref()
                    .and_then(|x| x.components().last())
                    .and_then(|x| x.as_os_str().to_str())
                    == Some(x)
            }) {
                try_set_all(process, set_low_priority);
            } else if FORCE_ASSIGN_NORMAL_SCHEDULER {
                try_set_all(process, set_normal);
            }
        }
    }
}
