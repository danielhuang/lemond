#![feature(once_cell)]
#![feature(exit_status_error)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub enum Message {
    SetForegroundPid { pid: Option<u32> },
    SetClientPid { pid: u32 },
}

pub mod proc;
pub mod procmon;
