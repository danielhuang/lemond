use std::{hint::black_box, sync::atomic::AtomicUsize, thread, time::Instant};

fn main() {
    let total = AtomicUsize::new(0);
    let start = Instant::now();
    let amount = 128;

    thread::scope(|s| {
        for _ in 0..8 {
            s.spawn(|| {
                let mut bufs = Vec::with_capacity(1024 * 1024);

                while start.elapsed().as_secs() < 45 {
                    let v = vec![1u8; amount * 1024 * 1024];
                    black_box(&v);
                    bufs.push(v);
                    let total = total.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    println!(
                        "{} GB used in {:?}",
                        (total * amount) / 1024,
                        start.elapsed()
                    );
                }
            });
        }
    });
    println!("out of time");
}
