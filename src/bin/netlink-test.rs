use lemond::{proc::ProcessHandle, procmon::ProcMon};

fn main() {
    let mon = ProcMon::new();
    loop {
        let e = mon.wait_for_event();
        let proc = ProcessHandle::from_pid(e.pid);
        if let Ok(proc) = proc {
            dbg!(&e, &proc);
            if let Ok(status) = proc.status() {
                dbg!(status.extract_num("VmData:"));
                dbg!(status.extract_num("VmSwap:"));
            }
        }
    }
}
