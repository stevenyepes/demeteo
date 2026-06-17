use crate::ports::execution::InteractiveHandle;
use ssh2::Channel;
use std::io;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

/// Remote SSH transport. Owns a long-lived `ssh2::Channel` over which the
/// remote agent's stdio is multiplexed. The channel is opened via
/// `Channel::exec(command)` so the agent inherits the user's shell env
/// and runs as if launched from an interactive SSH session.
pub struct RemoteSshTransport {
    channel: Arc<Mutex<Channel>>,
    _session: ssh2::Session,
    _tcp: std::net::TcpStream,
    cmd: String,
}

impl RemoteSshTransport {
    /// Wrap an already-opened `ssh2::Channel` in our transport.
    pub fn new(channel: Channel, session: ssh2::Session, tcp: std::net::TcpStream, cmd: String) -> Self {
        Self {
            channel: Arc::new(Mutex::new(channel)),
            _session: session,
            _tcp: tcp,
            cmd,
        }
    }

    fn drain_stderr(ch: &mut Channel) {
        let mut buf = [0u8; 1024];
        loop {
            match ch.stderr().read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if let Ok(s) = std::str::from_utf8(&buf[..n]) {
                        let trimmed = s.trim_end();
                        if !trimmed.is_empty() {
                            eprintln!("[remote agent stderr] {}", trimmed);
                        }
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
    }
}

impl InteractiveHandle for RemoteSshTransport {
    fn write_line(&self, line: &str) -> io::Result<usize> {
        let mut ch = self
            .channel
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "SSH channel lock poisoned"))?;
        let bytes = line.as_bytes();
        let mut written = 0;
        while written < bytes.len() {
            match ch.write(&bytes[written..]) {
                Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "failed to write whole line to SSH")),
                Ok(n) => {
                    written += n;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(e) => return Err(e),
            }
        }
        loop {
            match ch.flush() {
                Ok(()) => break,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(e) => return Err(e),
            }
        }
        Ok(written)
    }

    fn read_byte(&self) -> io::Result<u8> {
        let mut buf = [0u8; 1];
        let mut ch = self
            .channel
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "SSH channel lock poisoned"))?;
        
        Self::drain_stderr(&mut ch);

        match ch.read(&mut buf) {
            Ok(0) => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "ssh channel closed",
            )),
            Ok(_) => Ok(buf[0]),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::yield_now();
                Self::drain_stderr(&mut ch);
                match ch.read(&mut buf) {
                    Ok(0) => Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "ssh channel closed",
                    )),
                    Ok(_) => Ok(buf[0]),
                    Err(e2) if e2.kind() == io::ErrorKind::WouldBlock => {
                        Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "ssh channel has no data",
                        ))
                    }
                    Err(e2) => Err(e2),
                }
            }
            Err(e) => Err(e),
        }
    }

    fn try_read(&self, buf: &mut [u8]) -> io::Result<usize> {
        let mut ch = self
            .channel
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "SSH channel lock poisoned"))?;
        
        Self::drain_stderr(&mut ch);

        match ch.read(buf) {
            Ok(n) => Ok(n),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(0),
            Err(e) => Err(e),
        }
    }

    fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let mut ch = self
                .channel
                .lock()
                .map_err(|_| io::Error::new(io::ErrorKind::Other, "SSH channel lock poisoned"))?;
            
            Self::drain_stderr(&mut ch);

            match ch.read(buf) {
                Ok(0) => return Ok(0),
                Ok(n) => return Ok(n),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    drop(ch);
                    std::thread::yield_now();
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn kill(&self) -> Result<(), String> {
        let mut ch = self
            .channel
            .lock()
            .map_err(|_| "SSH channel lock poisoned")?;
        let _ = ch.send_eof();
        ch.close()
            .map_err(|e| format!("Failed to close SSH channel: {}", e))?;
        Ok(())
    }

    fn try_wait(&self) -> Result<Option<i32>, String> {
        let ch = self
            .channel
            .lock()
            .map_err(|_| "SSH channel lock poisoned")?;
        if ch.eof() {
            Ok(Some(0))
        } else {
            Ok(None)
        }
    }
}

impl Drop for RemoteSshTransport {
    fn drop(&mut self) {
        if let Ok(mut ch) = self.channel.lock() {
            let _ = ch.send_eof();
            let _ = ch.close();
        }
    }
}
