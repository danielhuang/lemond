use nix::{
    sys::{
        ptrace::{self, Options},
        wait::waitpid,
    },
    unistd::Pid,
};

pub fn exec_syscall(pid: i32) {
    let pid = Pid::from_raw(pid);
    ptrace::attach(pid).unwrap();
    // ptrace::interrupt(pid).unwrap();
    dbg!(waitpid(pid, None).unwrap());
    ptrace::syscall(pid, None).unwrap();
    dbg!(waitpid(pid, None).unwrap());
    let regs = ptrace::getregs(pid).unwrap();
    dbg!(&regs);
    ptrace::detach(pid, None).unwrap();
}
