#![feature(hash_drain_filter, once_cell, try_blocks)]

use cancellable_timer::Timer;
use color_eyre::eyre::Result;
use color_eyre::{Help, Report};
use lemond::proc::{get_all_pids, get_info_for_pid};
use lemond::procmon::ProcMon;
use lemond::{proc::ProcessHandle, Message};
use libc::{mlockall, setpriority, MCL_CURRENT, MCL_FUTURE};
use signal_hook::consts::signal::*;
use signal_hook::iterator::Signals;
use std::collections::HashSet;
use std::env::set_var;
use std::fs::{read_to_string, remove_file, File};
use std::io::Write;
use std::mem::size_of;
use std::os::unix::fs::PermissionsExt;
use std::process::exit;
use std::process::id;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
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

const REALTIME_PROCESSES: &[&str] = &[
    "/usr/bin/pipewire",
    "/usr/bin/pipewire-pulse",
    "/usr/bin/pipewire-media-session",
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
];

const MLOCKALL_PROCESSES: &[&str] = &[
    "/usr/bin/gnome-shell",
    "/usr/lib/Xorg",
    "/usr/bin/Xwayland",
    "/usr/bin/gnome-system-monitor",
    "/usr/lib/systemd/systemd-oomd",
    "/usr/bin/easyeffects",
];

const LOW_PRIORITY_PROCESSES: &[&str] = &["rustc", "cc", "c++", "gcc", "g++", "makepkg", "cc1"];

const SCHED_IDLEPRIO: i32 = 5;

const SOCKET_PATH: &str = "/run/lemond.socket";

const REALTIME_NICE: i32 = -11;
const BOOST_NICE: i32 = -5;
const LOW_PRIORITY_NICE: i32 = 19;

const REALTIME_RR: bool = true;

const USE_THREAD_IDS: bool = true;

const FORCE_ASSIGN_NORMAL_SCHEDULER: bool = true;

const ENABLE_SOCKET: bool = true;
const ENABLE_MLOCK: bool = true;
const ENABLE_GDB_MLOCK: bool = true;

const ENABLE_CFS_TWEAKS: bool = true;

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

#[repr(C)]
struct sched_attr {
    size: u32,

    sched_policy: u32,
    sched_flags: u64,

    /* SCHED_NORMAL, SCHED_BATCH */
    sched_nice: i32,

    /* SCHED_FIFO, SCHED_RR */
    sched_priority: u32,

    /* SCHED_DEADLINE (nsec) */
    sched_runtime: u64,
    sched_deadline: u64,
    sched_period: u64,
}

fn set_nice(pid: u32, nice: i32) -> Result<()> {
    let result = unsafe { setpriority(libc::PRIO_PROCESS, pid as _, nice) };
    match result {
        0 => Ok(()),
        -1 => Err(Report::from(Error::last_os_error())),
        _ => unreachable!(),
    }
}

fn set_scheduler(pid: u32, policy: i32, nice: i32, prio: i32, reset_on_fork: bool) -> Result<()> {
    const NS_PER_MS: u64 = 1000 * 1000;

    set_nice(pid, nice)?;

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
    };

    let result = unsafe { syscall!(Sysno::sched_setattr, pid, &attr as *const _, 0) };
    result.map(|_| ()).map_err(|e| {
        Report::from(Error::from_raw_os_error(e.into_raw()))
            .with_note(|| format!("pid={pid} policy={policy} nice={nice} prio={prio}"))
    })
}

fn set_realtime(p: &ProcessHandle, pid: u32) -> Result<()> {
    let i = p
        .executable
        .as_ref()
        .and_then(|x| x.as_os_str().to_str())
        .and_then(|x| REALTIME_PROCESSES.iter().position(|&s| x == s))
        .unwrap_or(10);

    let is_lemond = id() == p.pid;

    if REALTIME_RR {
        set_scheduler(
            pid,
            libc::SCHED_RR,
            REALTIME_NICE,
            99 - i as i32,
            !is_lemond,
        )?;
    } else {
        set_scheduler(pid, libc::SCHED_OTHER, REALTIME_NICE, 0, !is_lemond)?;
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
            return true;
        }
    }
    let exe = &p.executable;
    if let Some(exe) = exe {
        REALTIME_PROCESSES.iter().any(|&x| exe.to_str() == Some(x))
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

    while running.load(Ordering::SeqCst) {
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

                update_all_processes_once(&state, &process_cache.lock().unwrap());
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

fn cfs_tweaks(revert: Option<Vec<usize>>) -> Result<Vec<usize>> {
    let cpus = num_cpus::get();
    let score = 40;

    let values = vec![
        ("/sys/kernel/debug/sched/latency_ns", 1000 * cpus * score),
        (
            "/sys/kernel/debug/sched/min_granularity_ns",
            400 * cpus * score,
        ),
        (
            "/sys/kernel/debug/sched/wakeup_granularity_ns",
            500 * cpus * score,
        ),
        (
            "/sys/kernel/debug/sched/idle_min_granularity_ns",
            750 * cpus * score,
        ),
        (
            "/sys/kernel/debug/sched/migration_cost_ns",
            1500 * cpus * score,
        ),
    ];

    if let Some(revert) = revert {
        assert!(revert.len() == values.len());

        for ((path, _), num) in values.into_iter().zip(revert) {
            let mut file = File::create(path)?;
            write!(&mut file, "{num}")?;
        }

        Ok(vec![])
    } else {
        let mut prev = vec![];

        for (path, num) in values {
            let prev_value = read_to_string(path)?;
            prev.push(prev_value.trim().parse()?);

            let mut file = File::create(path)?;
            write!(&mut file, "{num}")?;
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

    assert!(REALTIME_PROCESSES
        .iter()
        .collect::<HashSet<_>>()
        .is_superset(&MLOCKALL_PROCESSES.iter().collect::<HashSet<_>>()));

    unsafe {
        mlockall(MCL_CURRENT | MCL_FUTURE);
    }

    let cfs_tweaks_prev = if ENABLE_CFS_TWEAKS {
        handle_error(cfs_tweaks(None))
    } else {
        None
    };

    println!("started");

    thread::scope(|scope| {
        scope.spawn({
            || {
                signals.forever().next();
                println!("exiting, setting processes back to normal");

                running.store(false, Ordering::SeqCst);
                canceller.cancel().unwrap();
                drop(state.lock().unwrap());

                handle_error(cleanup());

                {
                    let mut process_cache = process_cache.lock().unwrap();
                    process_cache.clear();
                }

                if let Some(cfs_tweaks_prev) = cfs_tweaks_prev {
                    handle_error(cfs_tweaks(Some(cfs_tweaks_prev)));
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

            while running.load(Ordering::SeqCst) {
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
            while running.load(Ordering::SeqCst) {
                populate_process_cache(&process_cache);
                update_all_processes_once(&state.lock().unwrap(), &process_cache.lock().unwrap());
                let _ = timer.sleep(Duration::from_millis(5000));
            }
        });
    });
}

fn populate_process_cache(process_cache: &Mutex<HashMap<u32, Option<ProcessHandle>>>) {
    let mut process_cache = process_cache.lock().unwrap();
    for pid in get_all_pids() {
        process_cache
            .entry(pid)
            .or_insert_with(|| get_info_for_pid(pid).ok());
    }
    process_cache.drain_filter(|_, x| x.is_none());
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
        if should_be_realtime(process, Some(state)) || process.pid == id() {
            handle_error(set_all(process, set_realtime));
            if ENABLE_MLOCK {
                handle_error(process.lock_executable());
            }
            if ENABLE_GDB_MLOCK
                && MLOCKALL_PROCESSES
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
                handle_error(set_all(process, set_boosted));
            } else if LOW_PRIORITY_PROCESSES.iter().any(|&x| {
                process
                    .executable
                    .as_ref()
                    .and_then(|x| x.components().last())
                    .and_then(|x| x.as_os_str().to_str())
                    == Some(x)
            }) {
                handle_error(set_all(process, set_low_priority));
            } else if FORCE_ASSIGN_NORMAL_SCHEDULER {
                handle_error(set_all(process, set_normal));
            }
        }
    }
}
