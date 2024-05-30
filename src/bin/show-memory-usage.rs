use chrono::Local;
use color_eyre::eyre::Result;
use lemond::{
    extract_num,
    proc::{get_all_pids, get_info_for_pid},
};
use std::{cmp::Reverse, fs::read_to_string, thread, time::Duration};

fn read_mem(pid: u32) -> (usize, usize) {
    let file = read_to_string(format!("/proc/{pid}/status")).unwrap();
    (
        extract_num(&file, "VmData:").unwrap(),
        extract_num(&file, "VmSwap:").unwrap(),
    )
}

fn main() {
    println!("{}", Local::now().to_rfc3339());
    let mut pids: Vec<_> = get_all_pids()
        .filter_map(|x| get_info_for_pid(x).map_err(|e| dbg!(x, e)).ok())
        .map(|x| (read_mem(x.pid), x))
        .collect();
    pids.sort_by_key(|x| x.0 .0 + x.0 .1);
    let mut total = 0;
    for (mem_usage, handle) in pids {
        println!(
            "{} {} mem={} swap={} total={}",
            handle.pid,
            handle.executable.as_ref().unwrap().to_string_lossy(),
            mem_usage.0,
            mem_usage.1,
            mem_usage.0 + mem_usage.1,
        );
        total += mem_usage.0 + mem_usage.1;
    }
    dbg!(&total);
}
