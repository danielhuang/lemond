// https://gitlab.freedesktop.org/drm/amd/-/issues/2125#note_1708592
// https://bbs.archlinux.org/viewtopic.php?id=293028

use lemond::{alloc_fault_in, fault_in};
use libc::{mlock, sysconf, _SC_PAGESIZE};
use rayon::{iter::ParallelIterator, slice::ParallelSliceMut};
use rustix::{event::pause, fs::sync};
use std::{
    fs::{read_dir, read_to_string, write},
    hint::black_box,
    io,
    sync::Barrier,
    thread::{self, available_parallelism},
    time::Instant,
};

const MLOCK: bool = false;

fn main() {
    let mut total = 0;

    for dev in read_dir("/sys/bus/pci/devices/").unwrap() {
        let dev = dev.unwrap();
        let vram = dev.path().join("mem_info_vram_total");
        if let Ok(vram) = read_to_string(vram) {
            let vram: usize = vram.trim().parse().unwrap();
            dbg!(vram);
            total += vram;
        }
    }

    total += 1024 * 1024 * 1024;

    let thread_count = available_parallelism().unwrap().get();
    let barrier = &Barrier::new(thread_count);

    let start = Instant::now();

    dbg!(&total);

    thread::scope(|s| {
        s.spawn(|| {
            sync();
            println!("sync done, took {:?}", start.elapsed());
        });

        for i in 0..thread_count {
            s.spawn(move || {
                let buf = alloc_fault_in(total / thread_count);
                black_box(&buf);

                if MLOCK {
                    unsafe {
                        let ret = mlock(buf.as_ptr() as _, buf.len());
                        if ret != 0 {
                            dbg!(io::Error::last_os_error());
                        }
                    }
                }

                println!("allocated memory {i}, took {:?}", start.elapsed());

                barrier.wait();

                black_box(&buf);
                drop(buf);

                println!("freed memory {i}, took {:?}", start.elapsed());
            });
        }
    });

    println!("done!");
}
