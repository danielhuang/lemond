use std::{
    hint::black_box,
    sync::atomic::AtomicUsize,
    thread::{self, available_parallelism},
    time::Instant,
};

fn main() {
    thread::scope(|s| {
        for _ in 0..available_parallelism().unwrap().get() {
            s.spawn(|| {
                println!("allocating");
                let mut buf = vec![0u8; 64 * 1024 * 1024 * 1024];
                black_box(&buf);
                println!("filling");
                buf.fill(1);
                black_box(&buf);
            });
        }
    });
}
