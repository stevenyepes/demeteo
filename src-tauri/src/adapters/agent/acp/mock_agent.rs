//! Mock ACP agent used for integration tests. The "agent" is a small
//! shell script that emits canned JSON-RPC messages on stdout and
//! reads requests from stdin. Phase 7b's AcpRuntime is meant to drive
//! this; the test verifies the full lifecycle: `initialize` →
//! `session/new` → `session/prompt` returning the expected
//! `AgentEvent` stream (Text, ToolCall, TurnComplete).
//!
//! Build a temp file with the script content and run it with
//! `LocalSubprocessTransport::spawn`.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

pub struct MockAgent {
    pub path: PathBuf,
}

impl MockAgent {
    /// Write a minimal "agent" that:
    /// 1. Reads and discards the `initialize` request, responds with
    ///    `{protocolVersion: 1}`.
    /// 2. Reads and discards the `session/new` request, responds with
    ///    `{sessionId: "mock-session-1"}`.
    /// 3. Reads the `session/prompt` request, then emits:
    ///    - one `session/update` text notification ("hello from mock")
    ///    - one `session/update` tool_call notification
    ///    - one final response with stop_reason "end_of_turn"
    /// 4. Exits.
    pub fn install() -> Self {
        Self::install_with_script(DEFAULT_SCRIPT)
    }

    /// Install a custom script. Used by the tool-call round-trip test
    /// to drive a richer state machine (read request, observe the
    /// client's `tool_call/update`, etc.).
    pub fn install_with_script(script: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "demeteo_mock_agent_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mock-acp-agent.sh");
        fs::write(&path, script).unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
        Self { path }
    }

    pub fn binary(&self) -> &str {
        self.path.to_str().unwrap()
    }
}

const DEFAULT_SCRIPT: &str = r#"#!/bin/sh
# Minimal ACP mock agent. Reads one line at a time from stdin, replies
# with the right JSON-RPC response. We use a tiny state machine driven
# by the call order. Late requests (tool_call/update after the prompt
# response, cancel, etc.) are acked with the request's own id so the
# client can resolve its oneshot.
state="init"
while IFS= read -r line; do
  case "$state" in
    init)
      echo '{"id":1,"result":{"protocolVersion":1}}'
      state="new"
      ;;
    new)
      echo '{"id":2,"result":{"sessionId":"mock-session-1"}}'
      state="prompted"
      ;;
    prompted)
      echo '{"method":"session/update","params":{"kind":"text","delta":"hello from mock"}}'
      echo '{"method":"session/update","params":{"kind":"tool_call","toolCallId":"tc-mock-1","action":"read","target":"/tmp/x"}}'
      echo '{"id":3,"result":{"stopReason":"end_of_turn"}}'
      state="idle"
      ;;
    *)
      # Late requests: echo the request's id. We pull "id" out of the
      # request body with sed so the client can resolve its oneshot
      # regardless of the actual id number.
      rid=$(echo "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
      if [ -z "$rid" ]; then rid=99; fi
      echo "{\"id\":$rid,\"result\":{}}"
      ;;
  esac
done
"#;

impl Drop for MockAgent {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        if let Some(parent) = self.path.parent() {
            let _ = fs::remove_dir(parent);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_agent_script_is_executable() {
        let m = MockAgent::install();
        let path = std::path::Path::new(m.binary());
        assert!(path.exists());
        let meta = std::fs::metadata(path).unwrap();
        assert!(meta.permissions().mode() & 0o111 != 0, "mock agent not executable");
    }

    #[test]
    fn mock_agent_drives_state_machine() {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let m = MockAgent::install();
        let mut child = Command::new(m.binary())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        // Feed three requests, read three responses.
        let mut stdin = child.stdin.take().unwrap();
        for i in 1..=3 {
            writeln!(stdin, "{{\"jsonrpc\":\"2.0\",\"id\":{},\"method\":\"x\",\"params\":{{}}}}", i).unwrap();
        }
        drop(stdin);

        let output = child.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout.lines().collect();
        // The mock agent emits 5 lines for 3 requests:
        //   - init:     1 response
        //   - new:      1 response
        //   - prompt:   2 notifications + 1 response
        assert_eq!(lines.len(), 5, "expected 5 lines, got {}: {}", lines.len(), stdout);
        // First response: init.
        assert!(lines[0].contains("\"protocolVersion\":1"));
        // Second: session/new.
        assert!(lines[1].contains("\"sessionId\":\"mock-session-1\""));
        // Third iteration emits two notifications then a response.
        assert!(lines[2].contains("\"session/update\""));
        assert!(lines[3].contains("\"tool_call\""));
        assert!(lines[4].contains("\"stopReason\":\"end_of_turn\""));
    }
}
