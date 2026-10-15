//! Private stdio server watchdog; it never loads Smith configuration.

use std::ffi::{OsStr, OsString};

use anyhow::Result;

#[cfg(unix)]
pub(crate) async fn run(parent: u32, program: &OsStr, args: &[OsString]) -> Result<u8> {
    use std::io;
    use std::os::unix::process::ExitStatusExt;
    use std::process::Stdio;
    use std::time::Duration;

    use nix::sys::signal::{Signal, kill};
    use nix::unistd::{Pid, getppid};
    use tokio::process::Command;
    use tokio::signal::unix::{SignalKind, signal};

    // Install handlers before spawning so startup signals cannot orphan a server.
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut hangup = signal(SignalKind::hangup())?;
    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            eprintln!("{error}");
            return Ok(
                if error.kind() == io::ErrorKind::PermissionDenied
                    || error.raw_os_error() == Some(nix::errno::Errno::ENOEXEC as i32)
                {
                    126
                } else {
                    127
                },
            );
        }
    };
    let mut poll = tokio::time::interval(Duration::from_millis(100));
    let parent_gone = loop {
        tokio::select! {
            biased;
            _ = poll.tick() => {
                if getppid().as_raw() != parent as i32 {
                    break true;
                }
            }
            _ = terminate.recv() => break false,
            // Terminal signals already reach the server in our shared group.
            _ = interrupt.recv() => {},
            _ = hangup.recv() => {},
            status = child.wait() => {
                let status = status?;
                return Ok(status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(0)) as u8);
            }
        }
    };

    if let Some(pid) = child.id() {
        let _ = kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
    }
    let status = match tokio::time::timeout(Duration::from_millis(300), child.wait()).await {
        Ok(status) => status?,
        Err(_) => {
            child.start_kill()?;
            child.wait().await?
        }
    };
    Ok(if parent_gone {
        1
    } else {
        status
            .code()
            .unwrap_or_else(|| 128 + status.signal().unwrap_or(0)) as u8
    })
}

#[cfg(not(unix))]
pub(crate) async fn run(_parent: u32, _program: &OsStr, _args: &[OsString]) -> Result<u8> {
    anyhow::bail!("MCP watchdog requires Unix parent process monitoring")
}
