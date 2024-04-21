use std::{hint::black_box, time::Instant};

fn main() {
    let mut bufs = Vec::with_capacity(1024 * 1024);
    let start = Instant::now();
    let amount = 128;
    while start.elapsed().as_secs() < 45 {
        let v = vec![1u8; amount * 1024 * 1024];
        black_box(&v);
        bufs.push(v);
        println!(
            "{} GB used in {:?}",
            (bufs.len() * amount) / 1024,
            start.elapsed()
        );
    }
    println!("out of time");
}
