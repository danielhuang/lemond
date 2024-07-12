use std::{fs::File, io::Read, slice};

use color_eyre::eyre::Result;
use rustix::{
    event::{poll, PollFd, PollFlags},
    fd::OwnedFd,
    fs::{open, Mode, OFlags},
    io::write,
};

// https://docs.kernel.org/accounting/psi.html

#[derive(Default)]
pub struct MemoryPressure {
    some_total_prev: usize,
    full_total_prev: usize,
    buf: String,
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Ord, Eq, Debug)]
pub struct MemoryPressureDelta {
    pub some: usize,
    pub full: usize,
}

impl MemoryPressure {
    pub fn new() -> Self {
        let mut x = Self {
            some_total_prev: 0,
            full_total_prev: 0,
            buf: String::new(),
        };
        x.buf.reserve(1024);
        x.read();
        x
    }

    pub fn read(&mut self) -> MemoryPressureDelta {
        self.buf.clear();
        File::open("/proc/pressure/memory")
            .unwrap()
            .read_to_string(&mut self.buf)
            .unwrap();

        fn get(buf: &str, key: &str) -> usize {
            let line = buf.lines().find(|x| x.contains(key)).unwrap();
            let (_, total) = line.rsplit_once("=").unwrap();
            total.parse().unwrap()
        }

        let some_total = get(&self.buf, "some");
        let full_total = get(&self.buf, "full");

        let result = MemoryPressureDelta {
            some: some_total - self.some_total_prev,
            full: full_total - self.full_total_prev,
        };

        self.some_total_prev = some_total;
        self.full_total_prev = full_total;

        result
    }
}

pub struct PollPressure {
    file: OwnedFd,
}

impl PollPressure {
    fn new(threshold: usize, total: usize, key: &str) -> Self {
        let file = open(
            "/proc/pressure/memory",
            OFlags::RDWR | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .unwrap();
        write(&file, format!("{key} {threshold} {total}\0").as_bytes()).unwrap();
        Self { file }
    }

    pub fn some(threshold: usize, total: usize) -> Self {
        Self::new(threshold, total, "some")
    }

    pub fn full(threshold: usize, total: usize) -> Self {
        Self::new(threshold, total, "full")
    }
}

impl PollPressure {
    pub fn wait(&mut self) {
        let mut fds = PollFd::new(&self.file, PollFlags::PRI);
        poll(slice::from_mut(&mut fds), -1).unwrap();
        assert!(!fds.revents().contains(PollFlags::ERR));
        assert!(fds.revents().contains(PollFlags::PRI));
    }
}
