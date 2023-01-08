use lemond::procmon::ProcMon;

fn main() {
    let mon = ProcMon::new();
    loop {
        dbg!(mon.wait_for_event());
    }
}
