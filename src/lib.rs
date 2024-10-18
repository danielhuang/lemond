use std::ptr::write_volatile;

use color_eyre::eyre::Result;
use libc::{madvise, sysconf, _SC_PAGESIZE};
use memmap::{MmapMut, MmapOptions};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub enum Message {
    SetForegroundPid { pid: Option<u32> },
    SetClientPid { pid: u32 },
}

pub fn extract_num(s: &str, prefix: &str) -> Option<usize> {
    let line = s.lines().find(|x| x.starts_with(prefix))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

#[track_caller]
pub fn handle_error<T>(r: Result<T>) -> Option<T> {
    if let Err(e) = &r {
        println!("{e:?}");
    }
    r.ok()
}

pub fn fault_in(mem: &mut [u8]) {
    let page_size = unsafe { sysconf(_SC_PAGESIZE) };
    for i in (0..mem.len()).step_by(page_size as usize) {
        unsafe { write_volatile(&mut mem[i] as *mut _, 0u8) };
    }
}

pub fn alloc_fault_in(min_len: usize) -> MmapMut {
    let page_size = unsafe { sysconf(_SC_PAGESIZE) } as usize;
    let len = (min_len + page_size - 1) / page_size * page_size;

    let mut mmap = MmapOptions::new().len(len).map_anon().unwrap();
    unsafe { madvise(mmap.as_mut_ptr() as *mut _, len, libc::MADV_POPULATE_WRITE) };

    mmap
}

pub mod config;
pub mod oom;
pub mod proc;
pub mod procmon;
pub mod trace;
pub mod zram_util;

#[cfg(debug_assertions)]
pub const DEBUG: bool = true;

#[cfg(not(debug_assertions))]
pub const DEBUG: bool = false;
