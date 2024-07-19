use std::{fs::File, io::Read, path::PathBuf, slice};

use color_eyre::eyre::Result;
use rustix::{
    event::{poll, PollFd, PollFlags},
    fd::{FromRawFd, IntoRawFd, OwnedFd},
    fs::{open, Mode, OFlags},
    io::write,
    path::Arg,
};
use std::io::Write;

// https://docs.kernel.org/accounting/psi.html

#[derive(Default)]
pub struct PsiReader {
    path: PathBuf,
    some_total_prev: usize,
    full_total_prev: usize,
    buf: String,
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Ord, Eq, Debug)]
pub struct PsiDelta {
    pub some: usize,
    pub full: usize,
}

impl PsiReader {
    pub fn new(path: PathBuf) -> Self {
        let mut x = Self {
            some_total_prev: 0,
            full_total_prev: 0,
            buf: String::new(),
            path,
        };
        x.buf.reserve(1024);
        x.read();
        x
    }

    pub fn memory() -> Self {
        Self::new("/proc/pressure/memory".into())
    }

    pub fn read(&mut self) -> PsiDelta {
        self.buf.clear();
        File::open(&self.path)
            .unwrap()
            .read_to_string(&mut self.buf)
            .unwrap();

        fn get(buf: &str, key: &str) -> usize {
            let line = buf.lines().find(|x| x.contains(key)).unwrap();
            let (_, total) = line.rsplit_once('=').unwrap();
            total.parse().unwrap()
        }

        let some_total = get(&self.buf, "some");
        let full_total = get(&self.buf, "full");

        let result = PsiDelta {
            some: some_total - self.some_total_prev,
            full: full_total - self.full_total_prev,
        };

        self.some_total_prev = some_total;
        self.full_total_prev = full_total;

        result
    }
}

pub struct PsiPoll {
    file: OwnedFd,
}

#[derive(Clone, Copy, Eq, PartialEq, PartialOrd, Ord, Debug)]
pub enum PsiLevel {
    Some,
    Full,
}

impl PsiPoll {
    fn new(threshold: usize, total: usize, level: PsiLevel, path: impl Arg) -> Self {
        let file = open(path, OFlags::RDWR | OFlags::NONBLOCK, Mode::empty()).unwrap();
        let key = match level {
            PsiLevel::Some => "some",
            PsiLevel::Full => "full",
        };
        write(&file, format!("{key} {threshold} {total}\0").as_bytes()).unwrap();
        Self { file }
    }

    pub fn memory(threshold: usize, total: usize, level: PsiLevel) -> Self {
        Self::new(threshold, total, level, "/proc/pressure/memory")
    }
}

impl PsiPoll {
    pub fn wait(&mut self) {
        let mut fds = PollFd::new(&self.file, PollFlags::PRI);
        poll(slice::from_mut(&mut fds), -1).unwrap();
        assert!(!fds.revents().contains(PollFlags::ERR));
        assert!(fds.revents().contains(PollFlags::PRI));
    }
}
