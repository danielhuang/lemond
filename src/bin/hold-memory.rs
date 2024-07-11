use std::hint::black_box;

use rand::{thread_rng, RngCore};
use rayon::{iter::ParallelIterator, slice::ParallelSliceMut};
use rustix::event::pause;

fn main() {
    let mut v = vec![0u8; 32 * 1024 * 1024 * 1024];
    println!("allocated");
    v.as_mut_slice()
        .par_chunks_mut(1024 * 1024 * 1024)
        .for_each(|v| {
            thread_rng().fill_bytes(v);
        });
    println!("filled");
    pause();
    black_box(&v);
}
