use std::{
    collections::HashMap,
    fs::{read_dir, write},
    path::Path,
    thread::{self, JoinHandle},
};

use color_eyre::eyre::{Context, Result};
use rustix::path::Arg;

use crate::{
    handle_error,
    oom::{PsiLevel, PsiPoll},
};

pub fn handle_device(device_path: &Path) -> Result<()> {
    let mut poll_pressure = PsiPoll::memory(300000, 1000000, PsiLevel::Full);

    let device_name = device_path
        .file_name()
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();

    loop {
        println!("writing {device_name} pages to disk");

        // declare all pages as idle...
        handle_error(write(device_path.join("idle"), "all").wrap_err("zram"));
        // then write to disk
        handle_error(write(device_path.join("writeback"), "idle").wrap_err("zram"));

        println!("device {device_name} write complete");

        poll_pressure.wait();
    }
}

pub fn init() {
    let mut poll_pressure = PsiPoll::memory(100000, 1000000, PsiLevel::Full);

    let mut device_map: HashMap<_, JoinHandle<_>> = HashMap::new();

    loop {
        poll_pressure.wait();

        let devices = read_dir("/sys/block").unwrap();

        device_map.retain(|_, x| !x.is_finished());

        for device in devices {
            let device = device.unwrap();

            let device_name = device
                .path()
                .file_name()
                .unwrap()
                .as_str()
                .unwrap()
                .to_string();

            if device_name.starts_with("zram") {
                device_map.entry(device_name).or_insert_with(|| {
                    let device_path = device.path();
                    thread::spawn(move || handle_error(handle_device(&device_path)))
                });
            }
        }
    }
}
