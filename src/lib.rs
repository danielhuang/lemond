use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub enum Message {
    SetForegroundPid { pid: Option<u32> },
    SetClientPid { pid: u32 },
}

pub mod oom;
pub mod proc;
pub mod procmon;
