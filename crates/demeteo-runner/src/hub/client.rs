//! A client of `serve`'s control socket, one NDJSON request per line, as
//! `rpc::serve` answers them.

use serde_json::{json, Value};
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;

pub struct ControlClient {
    lines: Lines<BufReader<OwnedReadHalf>>,
    write: OwnedWriteHalf,
    next_id: u64,
}

impl ControlClient {
    pub async fn connect(path: &Path) -> Result<Self, String> {
        let stream = UnixStream::connect(path).await.map_err(|e| {
            format!(
                "cannot reach `demeteo-runner serve` at {}: {e}. Is the runner running? \
                 (`systemctl --user status demeteo-runner`)",
                path.display()
            )
        })?;
        let (read, write) = stream.into_split();
        Ok(Self {
            lines: BufReader::new(read).lines(),
            write,
            next_id: 1,
        })
    }

    /// The method's `result`, or its `error` as `serve` worded it.
    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let mut line = json!({ "id": id, "method": method, "params": params }).to_string();
        line.push('\n');
        let lost = |e: std::io::Error| format!("lost the control socket: {e}");
        self.write.write_all(line.as_bytes()).await.map_err(lost)?;
        let answer = self
            .lines
            .next_line()
            .await
            .map_err(lost)?
            .ok_or_else(|| "the runner closed the control socket".to_owned())?;
        let mut answer: Value = serde_json::from_str(&answer)
            .map_err(|e| format!("the runner sent an unreadable answer: {e}"))?;
        if let Some(error) = answer.get("error").and_then(Value::as_str) {
            return Err(error.to_owned());
        }
        Ok(answer
            .get_mut("result")
            .map(Value::take)
            .unwrap_or(Value::Null))
    }
}
