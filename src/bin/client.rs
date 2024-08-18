use color_eyre::eyre::{eyre, Result};
use lemond::config::SOCKET_PATH;
use std::process::{id, Command};
use std::{os::unix::net::UnixDatagram, thread, time::Duration};

use lemond::Message;

fn run() -> Result<()> {
    let socket = UnixDatagram::unbound()?;
    socket.connect(SOCKET_PATH)?;

    let client_pid = id();
    socket.send(serde_json::to_string(&Message::SetClientPid { pid: client_pid })?.as_bytes())?;

    match get_foreground_pid() {
        Ok(foreground_pid) => {
            socket.send(
                serde_json::to_string(&Message::SetForegroundPid {
                    pid: Some(foreground_pid),
                })?
                .as_bytes(),
            )?;
        }
        Err(e) => {
            println!("{e:?}");
            socket.send(
                serde_json::to_string(&Message::SetForegroundPid { pid: None })?.as_bytes(),
            )?;
        }
    };

    Ok(())
}

fn get_foreground_pid() -> Result<u32> {
    let c = Command::new("gdbus")
        .arg("call")
        .arg("--session")
        .arg("--dest")
        .arg("org.gnome.Shell")
        .arg("--object-path")
        .arg("/org/gnome/Shell/Extensions/lemond")
        .arg("--method")
        .arg("com.lemond.ActiveWindowPid")
        .output()?
        .stdout;

    let output = String::from_utf8(c)?;

    let foreground_pid: u32 = output
        .trim()
        .strip_prefix("('")
        .ok_or_else(|| eyre!("error {output}"))?
        .strip_suffix("',)")
        .ok_or_else(|| eyre!("error {output}"))?
        .parse()?;

    Ok(foreground_pid)
}

fn main() {
    loop {
        if let Err(e) = run() {
            println!("{e:?}");
        }
        thread::sleep(Duration::from_millis(250));
    }
}
