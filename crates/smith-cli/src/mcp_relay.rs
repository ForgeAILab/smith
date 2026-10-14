//! Byte transport for the private turn socket, independent of MCP and setup.

use std::path::Path;

#[cfg(unix)]
use anyhow::Context;
use anyhow::Result;
#[cfg(unix)]
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::net::Shutdown;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(unix)]
use std::thread;

#[cfg(unix)]
pub(crate) fn run(path: &Path) -> Result<()> {
    let mut socket = UnixStream::connect(path)
        .with_context(|| format!("connecting to relay socket `{}`", path.display()))?;
    let mut writer = socket.try_clone()?;

    // The socket reader owns completion. Joining the stdin thread would keep
    // the relay alive after the server closes while stdin is still open.
    thread::Builder::new()
        .name("mcp-relay-stdin".into())
        .spawn(move || {
            let _ = io::copy(&mut io::stdin().lock(), &mut writer);
            // Half-closing preserves responses the server sends after input ends.
            let _ = writer.shutdown(Shutdown::Write);
        })?;

    let mut stdout = io::stdout().lock();
    let mut buffer = [0_u8; 8192];
    loop {
        let count = match socket.read(&mut buffer) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            // A peer closing with unread input can reset the stream instead
            // of returning EOF; both mean the relay's socket has closed.
            Err(error) if error.kind() == io::ErrorKind::ConnectionReset => return Ok(()),
            result => result?,
        };
        if count == 0 {
            return Ok(());
        }
        stdout.write_all(&buffer[..count])?;
        stdout.flush()?;
    }
}

#[cfg(not(unix))]
pub(crate) fn run(_path: &Path) -> Result<()> {
    anyhow::bail!("the MCP relay requires Unix sockets")
}
