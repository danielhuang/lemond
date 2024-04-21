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
    count: usize,
    buf: String,
}

impl MemoryPressure {
    pub fn new() -> Self {
        let mut x = Self::default();
        x.buf.reserve(1024);
        x.read();
        x
    }

    pub fn read(&mut self) -> usize {
        self.buf.clear();
        File::open("/proc/pressure/memory")
            .unwrap()
            .read_to_string(&mut self.buf)
            .unwrap();
        let full = self.buf.lines().find(|x| x.contains("full")).unwrap();
        let total = full.split_whitespace().last().unwrap();
        let (_, total) = total.split_once('=').unwrap();
        let total: usize = total.parse().unwrap();
        let result = total - self.count;
        self.count = total;
        result
    }
}

pub struct PollPressure {
    file: OwnedFd,
}

impl Default for PollPressure {
    fn default() -> Self {
        let file = open(
            "/proc/pressure/memory",
            OFlags::RDWR | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .unwrap();
        write(&file, b"full 500000 1000000\0").unwrap();
        Self { file }
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
