use chrono::Local;
use color_eyre::eyre::Result;
use lemond::{
    extract_num,
    proc::{get_all_pids, ProcessHandle},
};
use std::{cmp::Reverse, fs::read_to_string, thread, time::Duration};

fn read_mem(p: &ProcessHandle) -> (usize, usize) {
    let status = p.status().unwrap();
    (
        status.extract_num("VmRSS:").unwrap(),
        status.extract_num("VmSwap:").unwrap(),
    )
}

fn main() {
    println!("{}", Local::now().to_rfc3339());
    let mut pids: Vec<_> = get_all_pids()
        .filter_map(|x| ProcessHandle::from_pid(x).map_err(|e| dbg!(x, e)).ok())
        .map(|x| (read_mem(&x), x))
        .collect();
    pids.sort_by_key(|x| x.0 .0 + x.0 .1);
    let mut total = 0;
    for (mem_usage, handle) in pids {
        println!(
            "pid={} {} mem={:.1}GB swap={:.1}GB total={:.1}GB",
            handle.pid,
            handle.executable.as_ref().unwrap(),
            mem_usage.0 as f64 / 1000000.0,
            mem_usage.1 as f64 / 1000000.0,
            (mem_usage.0 + mem_usage.1) as f64 / 1000000.0,
        );
        total += mem_usage.0 + mem_usage.1;
    }
    println!();
    let kernel = read_to_string("/proc/sys/kernel/osrelease").unwrap();
    let kernel = kernel.trim();
    let swappiness = read_to_string("/proc/sys/vm/swappiness").unwrap();
    let swappiness = swappiness.trim();
    println!(
        "kernel={kernel} swappiness={swappiness} total={:.1}GB",
        total as f64 / 1000000.0
    );
}
