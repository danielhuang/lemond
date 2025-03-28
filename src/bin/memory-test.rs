use std::{
    hint::black_box,
    sync::atomic::AtomicUsize,
    thread::{self, available_parallelism},
    time::Instant,
};

use lemond::{alloc_fault_in, fault_in};

fn main() {
    let total = AtomicUsize::new(0);
    let start = Instant::now();
    let amount = 128;

    thread::scope(|s| {
        for _ in 0..available_parallelism().unwrap().get() {
            s.spawn(|| {
                let mut bufs = Vec::with_capacity(1024 * 1024);

                while start.elapsed().as_secs() < 180 {
                    let mut v = vec![0u8; amount * 1024 * 1024];
                    v.fill(1);
                    black_box(&v);
                    bufs.push(v);
                    let total = total.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    println!(
                        "{} GB used in {:?}",
                        (total * amount) as f64 / 1024.0,
                        start.elapsed()
                    );
                }
            });
        }
    });

    println!("reached time limit");
}
