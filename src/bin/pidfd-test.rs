use std::{fs::File, io::Read, os::fd::AsFd, process::id};

use rustix::{
    event::pause,
    fs::{open, openat, Mode, OFlags},
    process::{pidfd_open, pidfd_send_signal, Pid, PidfdFlags},
};

fn main() {
    let fd = open(format!("/proc/{}", id()), OFlags::DIRECTORY, Mode::empty()).unwrap();
    let file = openat(fd.as_fd(), "status", OFlags::empty(), Mode::empty()).unwrap();
    let mut file = File::from(file);
    let mut s = String::new();
    file.read_to_string(&mut s).unwrap();
    dbg!(&s);
    pidfd_send_signal(fd, rustix::process::Signal::Kill).unwrap();
    pause();
}
