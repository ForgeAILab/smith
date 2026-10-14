//! Private process lifetime watchdog; it never loads Smith configuration.

use std::ffi::OsStr;
use std::ffi::OsString;

use anyhow::Result;

#[cfg(unix)]
pub(crate) async fn run(program: &OsStr, args: &[OsString]) -> Result<u8> {
    use std::io::{self, Read};
    use std::os::unix::process::ExitStatusExt;
    use std::process::Stdio;

    use nix::sys::signal::{Signal, killpg};
    use nix::unistd::getpgrp;
    use tokio::process::Command;
    use tokio::signal::unix::{SignalKind, signal};
    use tokio::sync::oneshot;

    // Register before spawning: a signal during startup must also stop the child.
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    let mut hangup = signal(SignalKind::hangup())?;
    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        // The child must inherit our group, so Smith's kill reaches descendants.
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

    let (closed, eof) = oneshot::channel();
    // A detached std thread avoids Tokio's blocking-stdin shutdown wait when
    // the child finishes while Smith still holds the watchdog pipe open.
    std::thread::spawn(move || {
        let mut input = io::stdin().lock();
        let mut bytes = [0; 256];
        loop {
            match input.read(&mut bytes) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        let _ = closed.send(());
    });

    tokio::select! {
        biased;
        _ = interrupt.recv() => {},
        _ = terminate.recv() => {},
        _ = hangup.recv() => {},
        _ = eof => {},
        status = child.wait() => {
            let status = status?;
            return Ok(status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(0)) as u8);
        }
    }
    // Killing our own group includes this supervisor. There is no interval
    // between orphan cleanup and exit in which another child could survive.
    let _ = killpg(getpgrp(), Signal::SIGKILL);
    std::process::exit(1);
}

#[cfg(not(unix))]
pub(crate) async fn run(_program: &OsStr, _args: &[OsString]) -> Result<u8> {
    anyhow::bail!("harness supervision requires Unix process groups")
}
