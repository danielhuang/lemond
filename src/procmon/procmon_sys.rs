#[repr(C)]
#[derive(Default)]
pub struct Event {
    pub event_type: u32,
    pub pid: libc::pid_t,
    pub ppid: libc::pid_t,
    pub tgid: libc::pid_t,
}

#[link(name = "procmon")]
extern "C" {
    pub fn nl_connect() -> i32;
    pub fn update_nl(nl_socket: i32, enable: bool) -> i32;
    pub fn wait_for_event(nl_socket: i32, event: *mut Event) -> i32;
}
