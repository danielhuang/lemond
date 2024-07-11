use nix::{
    sys::wait::{waitid, waitpid, WaitPidFlag},
    unistd::{fork, write},
};
use rustix::fd::BorrowedFd;

fn main() {
    for _ in 0.. {
        let result = unsafe { fork() }.unwrap();
        match result {
            nix::unistd::ForkResult::Parent { child } => {
                waitid(nix::sys::wait::Id::Pid(child), WaitPidFlag::WEXITED).unwrap();
            }
            nix::unistd::ForkResult::Child => unsafe {
                write(BorrowedFd::borrow_raw(1), b"forked\n").unwrap();
            },
        }
    }
    unsafe {
        write(BorrowedFd::borrow_raw(1), b"done\n").unwrap();
    }
}
