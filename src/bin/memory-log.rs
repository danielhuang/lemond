use chrono::Local;
use color_eyre::eyre::Result;
use lemond::{
    extract_num,
    proc::{get_all_pids, get_info_for_pid},
};
use std::{cmp::Reverse, fs::read_to_string, thread, time::Duration};

fn read_mem(pid: u32) -> Result<(Option<usize>, Option<usize>)> {
    let file = read_to_string(format!("/proc/{pid}/status"))?;
    Ok((extract_num(&file, "VmData:"), extract_num(&file, "VmSwap:")))
}

fn main() {
    loop {
        println!("{}", Local::now().to_rfc3339());
        let mut pids: Vec<_> = get_all_pids()
            .map(get_info_for_pid)
            .filter_map(|x| x.ok())
            .map(|x| (read_mem(x.pid), x))
            .collect();
        pids.sort_by_key(|x| x.0.as_ref().ok().map(|x| x.0.map(Reverse)));
        for pid in pids {
            println!("{:?}", pid);
        }
        println!();
        thread::sleep(Duration::from_secs(10));
    }
}
