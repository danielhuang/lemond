use rustix::fd::{AsRawFd, FromRawFd, OwnedFd};

mod procmon_sys;

#[derive(Debug)]
pub struct ProcMon {
    fd: OwnedFd,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum EventType {
    Fork,
    Exec,
    Exit,
    Other,
}

#[derive(Debug, Clone)]
pub struct Event {
    pub event_type: Option<EventType>,
    pub pid: u32,
    pub ppid: u32,
    pub tgid: u32,
}

impl ProcMon {
    pub fn new() -> Self {
        unsafe {
            let fd: i32 = procmon_sys::nl_connect();
            procmon_sys::update_nl(fd, true);

            Self {
                fd: OwnedFd::from_raw_fd(fd),
            }
        }
    }

    pub fn wait_for_event(&self) -> Event {
        let mut event = procmon_sys::Event::default();

        unsafe {
            procmon_sys::wait_for_event(self.fd.as_raw_fd(), &mut event);
        };

        Event {
            event_type: match event.event_type {
                0 => None,
                1 => Some(EventType::Fork),
                2 => Some(EventType::Exec),
                0x80000000 => Some(EventType::Exit),
                _ => Some(EventType::Other),
            },
            pid: event.pid as _,
            ppid: event.ppid as _,
            tgid: event.tgid as _,
        }
    }
}

impl Default for ProcMon {
    fn default() -> Self {
        Self::new()
    }
}
