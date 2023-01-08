mod procmon_sys;

#[derive(Debug, Clone)]
pub struct ProcMon {
    nl_socket: i32,
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

fn map_int_to_event_type(i: u32) -> Option<EventType> {
    match i {
        0 => None,
        1 => Some(EventType::Fork),
        2 => Some(EventType::Exec),
        0x8000_0000 => Some(EventType::Exit),
        _ => Some(EventType::Other),
    }
}

impl ProcMon {
    pub fn new() -> Self {
        let nls: i32 = unsafe { procmon_sys::nl_connect() };
        unsafe {
            procmon_sys::update_nl(nls, true);
        }

        ProcMon { nl_socket: nls }
    }

    pub fn wait_for_event(&self) -> Event {
        let mut event = procmon_sys::Event {
            event_type: 0,
            pid: 0,
            ppid: 0,
            tgid: 0,
        };

        unsafe {
            procmon_sys::wait_for_event(self.nl_socket, &mut event);
        };

        Event {
            event_type: map_int_to_event_type(event.event_type),
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
