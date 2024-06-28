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

pub mod config;
pub mod oom;
pub mod proc;
pub mod procmon;
pub mod trace;
