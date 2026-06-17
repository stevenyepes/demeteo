use crate::ports::execution::InteractiveHandle;
use std::io;
use std::io::{Read, Write};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

/// Local subprocess transport: wraps a `std::process::Child` (blocking)
/// and exposes its stdin/stdout as blocking-style methods on the
/// `InteractiveHandle` trait. stderr is drained to a background thread
/// to avoid pipe-fill deadlocks for noisy agents.
pub struct LocalSubprocessTransport {
    child: Mutex<Child>,
    stdin: Mutex<Option<ChildStdin>>,
    stdout: Mutex<Option<ChildStdout>>,
    stderr_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl LocalSubprocessTransport {
    pub fn spawn(
        binary: &str,
        args: &[String],
        cwd: &str,
        env: &std::collections::HashMap<String, String>,
    ) -> Result<Self, String> {
        let mut cmd = Command::new(binary);
        cmd.args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to spawn '{}': {}", binary, e))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Failed to capture agent stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Failed to capture agent stdout".to_string())?;
        let stderr = child.stderr.take();
        let stderr_thread = stderr.map(spawn_stderr_drain);

        Ok(Self {
            child: Mutex::new(child),
            stdin: Mutex::new(Some(stdin)),
            stdout: Mutex::new(Some(stdout)),
            stderr_thread: Mutex::new(stderr_thread),
        })
    }
}

fn spawn_stderr_drain(mut stderr: ChildStderr) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if io::copy(&mut stderr, &mut buf).is_ok() && !buf.is_empty() {
            if let Ok(s) = std::str::from_utf8(&buf) {
                let trimmed = s.trim_end();
                if !trimmed.is_empty() {
                    eprintln!("[agent stderr] {}", trimmed);
                }
            }
        }
    })
}

impl InteractiveHandle for LocalSubprocessTransport {
    fn write_line(&self, line: &str) -> io::Result<usize> {
        let mut guard = self
            .stdin
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "stdin lock poisoned"))?;
        let stdin = guard
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "stdin closed"))?;
        stdin.write_all(line.as_bytes())?;
        stdin.flush()?;
        Ok(line.len())
    }

    fn read_byte(&self) -> io::Result<u8> {
        let mut buf = [0u8; 1];
        let mut guard = self
            .stdout
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "stdout lock poisoned"))?;
        let stdout = guard
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "stdout closed"))?;
        match stdout.read(&mut buf) {
            Ok(0) => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "agent closed stdout",
            )),
            Ok(_) => Ok(buf[0]),
            Err(e) => Err(e),
        }
    }

    fn try_read(&self, buf: &mut [u8]) -> io::Result<usize> {
        let mut guard = self
            .stdout
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "stdout lock poisoned"))?;
        let stdout = guard
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "stdout closed"))?;
        // The default blocking read; the JSON-RPC layer treats this as
        // "block until bytes or EOF". The watchdog kills us on hang.
        match stdout.read(buf) {
            Ok(n) => Ok(n),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => Ok(0),
            Err(e) => Err(e),
        }
    }

    fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        let mut guard = self
            .stdout
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "stdout lock poisoned"))?;
        let stdout = guard
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "stdout closed"))?;
        stdout.read(buf)
    }

    fn kill(&self) -> Result<(), String> {
        if let Ok(mut st) = self.stderr_thread.lock() {
            if let Some(t) = st.take() {
                // Best-effort: a blocking stderr drain thread will exit when
                // the pipe closes (which happens after kill).
                drop(t);
            }
        }
        let mut child = self.child.lock().map_err(|_| "child lock poisoned")?;
        child
            .kill()
            .map_err(|e| format!("Failed to kill agent: {}", e))?;
        Ok(())
    }

    fn try_wait(&self) -> Result<Option<i32>, String> {
        let mut child = self.child.lock().map_err(|_| "child lock poisoned")?;
        match child.try_wait() {
            Ok(Some(status)) => Ok(status.code()),
            Ok(None) => Ok(None),
            Err(e) => Err(format!("try_wait failed: {}", e)),
        }
    }
}

impl Drop for LocalSubprocessTransport {
    fn drop(&mut self) {
        // Best-effort kill on drop so the OS doesn't leave a zombie.
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn local_transport_spawns_and_round_trips() {
        let mut t = LocalSubprocessTransport::spawn(
            "sh",
            &[
                "-c".to_string(),
                "read line; echo \"got:$line\"".to_string(),
            ],
            ".",
            &HashMap::new(),
        )
        .expect("spawn");

        t.write_line("hello\n").expect("write");

        let mut got = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        while std::time::Instant::now() < deadline {
            match t.read_byte() {
                Ok(b) => {
                    got.push(b);
                    if b == b'\n' {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let s = String::from_utf8_lossy(&got);
        assert!(s.contains("got:hello"), "expected 'got:hello' in '{}'", s);

        t.kill().ok();
    }

    #[test]
    fn local_transport_try_wait_returns_none_while_running() {
        let mut t = LocalSubprocessTransport::spawn(
            "sh",
            &["-c".to_string(), "sleep 1".to_string()],
            ".",
            &HashMap::new(),
        )
        .expect("spawn");
        assert!(matches!(t.try_wait(), Ok(None)));
        t.kill().ok();
    }
}
