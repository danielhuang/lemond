use cancellable_timer::Timer;
use color_eyre::eyre::{Context, Result};
use color_eyre::{Help, Report};
use lemond::extract_num;
use lemond::oom::{MemoryPressure, PollPressure};
use lemond::proc::{get_all_pids, get_info_for_pid, process_mrelease};
use lemond::procmon::ProcMon;
use lemond::{proc::ProcessHandle, Message};
use libc::{
    kill, mlockall, mmap, pid_t, rlimit, rlimit64, setpriority, waitpid, MCL_CURRENT, MCL_FUTURE,
    RLIMIT_CPU, RLIMIT_NICE, RLIMIT_RTPRIO, RLIMIT_RTTIME, RLIM_INFINITY,
};
use rustix::fd::AsFd;
use rustix::fs::{openat, Mode, OFlags};
use rustix::process::{
    pidfd_open, pidfd_send_signal, prlimit, Pid, PidfdFlags, Resource, Rlimit, Signal,
};
use signal_hook::consts::signal::*;
use signal_hook::iterator::Signals;
use std::collections::HashSet;
use std::env::set_var;
use std::fs::{self, read_to_string, remove_file, File};
use std::io::{self, Read, Write};
use std::mem::size_of;
use std::os::unix::fs::PermissionsExt;
use std::process::id;
use std::process::{abort, exit};
use std::ptr::null_mut;
use std::sync::mpsc::sync_channel;
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
    "/usr/lib/mutter-x11-frames",
    "/usr/bin/plasma-systemmonitor",
];

const EXCLUDE_REALTIME: &[&str] = &[
    "/usr/bin/gnome-shell",
    "/usr/bin/Xwayland",
    "/usr/lib/mutter-x11-frames",
];

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
    "/usr/bin/Xwayland",
    "/usr/lib/mutter-x11-frames",
    "/usr/bin/plasma-systemmonitor",
];

const LOW_PRIORITY_PROCESSES: &[&str] = &["rustc", "cc", "c++", "gcc", "g++", "makepkg", "cc1"];

const SCHED_IDLEPRIO: i32 = 5;
const SCHED_DEADLINE: i32 = 6;

const SOCKET_PATH: &str = "/run/lemond.socket";

const REALTIME_NICE: i32 = -20;
const BOOST_NICE: i32 = -7;
const LOW_PRIORITY_NICE: i32 = 19;

const ENABLE_REALTIME: bool = true;

const USE_THREAD_IDS: bool = true;

const FORCE_ASSIGN_NORMAL_SCHEDULER: bool = true;

const ENABLE_NICE: bool = true;

const ENABLE_SOCKET: bool = true;
const ENABLE_EXE_MLOCK: bool = true;
const ENABLE_LOCK_FDS: bool = false;
const ENABLE_GDB_MLOCK: bool = true;

const ENABLE_KERNEL_TWEAKS: bool = true;

const LEMOND_SELF_REALTIME: bool = true;

use tikv_jemallocator::Jemalloc;

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

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
        .and_then(|x| CRITICAL_PROCESSES.iter().position(|&s| x == s))
        .unwrap_or(10);

    let is_lemond = id() == p.pid;

    if ENABLE_REALTIME
        && p.executable
            .as_ref()
            .is_some_and(|x| !EXCLUDE_REALTIME.contains(&x.as_str()))
    {
        let limit = rlimit64 {
            rlim_cur: RLIM_INFINITY,
            rlim_max: RLIM_INFINITY,
        };

        // workaround https://gitlab.freedesktop.org/drm/amd/-/issues/2861
        // also see ~/.config/environment.d/mutter.conf
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
    let e = set_all(p, f).with_note(|| format!("handle={p:?}"));
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
        CRITICAL_PROCESSES.iter().any(|&x| exe == x)
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
    extract_num(&info, "MemTotal:").unwrap()
}

fn available_memory_kb() -> usize {
    let info = read_to_string("/proc/meminfo").unwrap();
    extract_num(&info, "MemAvailable:").unwrap()
}

fn total_memory() -> usize {
    total_memory_kb() * 1024
}

fn kernel_tweaks(revert: Option<Vec<String>>) -> Result<Vec<String>> {
    let values: Vec<(&'static str, String)> = vec![
        // ("/proc/sys/vm/compaction_proactiveness", "0".to_string()),
        ("/proc/sys/vm/min_free_kbytes", "4194304".to_string()),
        //("/proc/sys/vm/swappiness", "0".to_string()),
        // ("/sys/kernel/mm/lru_gen/enabled", "5".to_string()),
        // ("/sys/kernel/mm/lru_gen/min_ttl_ms", "1000".to_string()),
        // ("/proc/sys/vm/zone_reclaim_mode", "0".to_string()),
        (
            "/sys/kernel/mm/transparent_hugepage/enabled",
            "madvise".to_string(),
        ),
        (
            "/sys/kernel/mm/transparent_hugepage/shmem_enabled",
            "advise".to_string(),
        ),
        (
            "/sys/kernel/mm/transparent_hugepage/defrag",
            "never".to_string(),
        ),
        // ("/proc/sys/vm/page_lock_unfairness", "1".to_string()),
        // ("/proc/sys/kernel/sched_child_runs_first", "0".to_string()),
        ("/proc/sys/kernel/sched_autogroup_enabled", "0".to_string()),
        // (
        //     "/proc/sys/kernel/sched_cfs_bandwidth_slice_us",
        //     "500".to_string(),
        // ),
        // (
        //     "/sys/kernel/debug/sched/migration_cost_ns",
        //     "500000".to_string(),
        // ),
        // ("/sys/kernel/debug/sched/nr_migrate", "8".to_string()),
        // ("/sys/power/image_size", "0".to_string()),
        // ("/sys/power/image_size", total_memory().to_string()),
        ("/proc/sys/vm/page-cluster", "0".to_string()),
        // ("/sys/module/zswap/parameters/enabled", "N".to_string()),
        (
            "/sys/module/zswap/parameters/shrinker_enabled",
            "N".to_string(),
        ),
        ("/sys/module/zswap/parameters/compressor", "lz4".to_string()),
        // (
        //     "/sys/module/zswap/parameters/max_pool_percent",
        //     "50".to_string(),
        // ),
        // https://github.com/pop-os/default-settings/blob/master_jammy/etc/sysctl.d/10-pop-default-settings.conf
        ("/proc/sys/vm/watermark_boost_factor", "0".to_string()),
        ("/proc/sys/vm/watermark_scale_factor", "125".to_string()),
        ("/proc/sys/vm/vfs_cache_pressure", "50".to_string()),
        ("/proc/sys/vm/dirty_bytes", "268435456".to_string()),
        (
            "/proc/sys/vm/dirty_background_bytes",
            "134217728".to_string(),
        ),
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
                file.flush().unwrap();
            }
        }

        Ok(vec![])
    } else {
        let mut prev = vec![];

        for (path, value) in values {
            if let Ok(mut file) = File::create(path) {
                let prev_value = read_to_string(path)?;
                println!("setting {path} to {value} (was {})", prev_value.trim());
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

    fs::write("/proc/self/oom_score_adj", "-1000").unwrap();

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
        if ENABLE_SOCKET {
            scope.spawn(|| {
                socket_handler(&state, &running, &process_cache);
            });
        }
        scope.spawn(|| {
            let mon = ProcMon::new();

            populate_process_cache(&mut process_cache.lock().unwrap());

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
                populate_process_cache(&mut process_cache.lock().unwrap());
                update_all_processes_once(&state.lock().unwrap(), &process_cache.lock().unwrap());
                let _ = timer.sleep(Duration::from_millis(5000));
            }
        });
        scope.spawn(|| {
            let mut buf = String::with_capacity(32 * 1024);

            let mut mp = MemoryPressure::new();
            let mut poll_pressure = PollPressure::new(800000, 1000000);

            let total_memory_kb = total_memory_kb();

            while running.load(Ordering::Relaxed) {
                mp.read();

                poll_pressure.wait();

                let pressure = mp.read();
                println!("memory pressure monitor event received (pressure={pressure})");

                if pressure == 0 {
                    println!("pressure not above threshold");
                    continue;
                }

                let free_memory_kb = {
                    buf.clear();
                    File::open("/proc/meminfo").unwrap().read_to_string(&mut buf).unwrap();
                    extract_num(&buf, "MemAvailable:").unwrap()
                };

                println!("available memory: {free_memory_kb} KB");

                let start = Instant::now();

                let cache = process_cache.lock().unwrap();

                let (&pid, handle, mem_used_kb) = cache
                    .iter()
                    .filter_map(|(pid, process)| {
                        if let Some(process) = process {
                            if !should_be_realtime(process, None) {
                                return Some((pid, process));
                            }
                        }
                        None
                    })
                    .map(|(pid, process)| {
                        if let Ok(mut file) =
                            openat(process.proc_dirfd.as_fd(), "status", OFlags::RDONLY, Mode::empty()).map(File::from)
                        {
                            buf.clear();
                            file.read_to_string(&mut buf).unwrap();
                            let mem_used_kb = extract_num(&buf, "VmData:");
                            let swap_used_kb = extract_num(&buf, "VmSwap:");
                            (pid, process, mem_used_kb.map(|x| x + swap_used_kb.unwrap_or(0)))
                        } else {
                            (pid, process, None)
                        }
                    })
                    .max_by_key(|(_, _, x)| *x)
                    .unwrap();

                let Ok(pidfd) = handle.pidfd.try_clone() else {
                    println!("failed to open pidfd (pid={pid})! does process not exist?");
                    continue;
                };

                println!(
                    "found target (pid={}, pidfd={pidfd:?} exe={:?} mem_used_kb={mem_used_kb:?}) took {:?} to find",
                    pid,
                    handle.executable,
                    start.elapsed(),
                );

                drop(cache);

                if free_memory_kb > (total_memory_kb / 10) {
                    println!("enough memory left, skipping kill");
                    continue;
                }

                let kill_start = Instant::now();

                handle_error(pidfd_send_signal(&pidfd, Signal::Kill).wrap_err(format!("pid={pid} pidfd={pidfd:?}")));
                handle_error(process_mrelease(&pidfd, 0));

                loop {
                    if pidfd_send_signal(&pidfd, Signal::Kill).is_err() {
                        println!("process killed in {:?} ({:?} total)", kill_start.elapsed(), start.elapsed());
                        break;
                    }
                    thread::yield_now();
                    if kill_start.elapsed().as_secs_f64() > 20.0 {
                        println!("kill is taking too long, bailing");
                        break;
                    }
                }
            }
        });
    });
}

fn populate_process_cache(process_cache: &mut HashMap<u32, Option<ProcessHandle>>) {
    let pids: HashSet<_> = get_all_pids().collect();
    for &pid in &pids {
        process_cache
            .entry(pid)
            .or_insert_with(|| get_info_for_pid(pid).ok());
    }
    process_cache.retain(|pid, handle| pids.contains(pid) && handle.is_some());
    process_cache.shrink_to_fit();
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
            if ENABLE_EXE_MLOCK {
                handle_error(process.lock_executable());
            }
            if ENABLE_LOCK_FDS {
                handle_error(process.lock_fds());
            }
            if ENABLE_GDB_MLOCK
                && GDB_MLOCK_PROCESSES
                    .iter()
                    .any(|&x| process.executable.as_deref() == Some(x))
            {
                handle_error(process.gdb_lock_all());
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
                    .and_then(|x| x.split('/').last())
                    == Some(x)
            }) {
                try_set_all(process, set_low_priority);
            } else if FORCE_ASSIGN_NORMAL_SCHEDULER {
                try_set_all(process, set_normal);
            }
        }
    }
}
