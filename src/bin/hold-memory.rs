use std::hint::black_box;

use rand::{thread_rng, Rng, RngCore};
use rustix::event::pause;

fn main() {
    let mut v = vec![0u8; 32 * 1024 * 1024 * 1024];
    println!("allocated");
    thread_rng().fill_bytes(&mut v);
    println!("filled");
    pause();
    black_box(&v);
}
