//! The scripted instance and the frames it names. Why every departure from a
//! script is a panic is on [the module](super).

use demeteo_hub_protocol::{CursorAck, SignedRequest, PROTOCOL_VERSION};
use serde::Serialize;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;

use super::transport::{ChannelTransport, Down, Up};

/// How long a script waits for a frame, and for the Hub end to go away,
/// unless [`FakeInstance::waiting_at_most`] says otherwise. Measured against
/// nothing: long enough that a loaded machine does not fail a correct
/// session, and paid in full only by a test that fails anyway.
pub const PATIENCE: Duration = Duration::from_secs(2);

/// A Hub frame as a script names one. `Display` gives the kind and the
/// number a script matched on, which is all a failure needs to say.
#[derive(Debug, PartialEq)]
enum HubFrame {
    Ack(CursorAck),
    Request(Box<SignedRequest>),
    Close(u16),
    Other { bytes: usize },
}

impl From<Down> for HubFrame {
    fn from(frame: Down) -> Self {
        match frame {
            Down::Close(code) => Self::Close(code),
            Down::Text(text) => {
                if let Ok(ack) = serde_json::from_str(&text) {
                    Self::Ack(ack)
                } else if let Ok(request) = serde_json::from_str(&text) {
                    Self::Request(Box::new(request))
                } else {
                    Self::Other { bytes: text.len() }
                }
            }
        }
    }
}

impl fmt::Display for HubFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ack(ack) => write!(
                f,
                "a cursor ack at offset {} (protocol version {})",
                ack.offset, ack.v
            ),
            Self::Request(_) => f.write_str("a signed request"),
            Self::Close(code) => write!(f, "a close with code {code}"),
            Self::Other { bytes } => write!(
                f,
                "{bytes} bytes of text that are neither a cursor ack nor a signed request"
            ),
        }
    }
}

pub struct FakeInstance {
    up: Option<mpsc::UnboundedSender<Up>>,
    down: mpsc::UnboundedReceiver<Down>,
    answering: Arc<AtomicBool>,
    reading: Arc<AtomicBool>,
    refusing: Arc<AtomicBool>,
    patience: Duration,
}

impl FakeInstance {
    /// An instance and the Hub end of its connection, already open: admission
    /// happens before there is a transport, so there is no handshake to play.
    pub fn connect() -> (Self, ChannelTransport) {
        let (up_tx, up_rx) = mpsc::unbounded_channel();
        let (down_tx, down_rx) = mpsc::unbounded_channel();
        let answering = Arc::new(AtomicBool::new(true));
        let reading = Arc::new(AtomicBool::new(true));
        let refusing = Arc::new(AtomicBool::new(false));
        let instance = Self {
            up: Some(up_tx),
            down: down_rx,
            answering: Arc::clone(&answering),
            reading: Arc::clone(&reading),
            refusing: Arc::clone(&refusing),
            patience: PATIENCE,
        };
        let hub = ChannelTransport {
            up: up_rx,
            down: down_tx,
            answering,
            reading,
            refusing,
            pongs: 0,
        };
        (instance, hub)
    }

    /// For a script that means to wait its bound out.
    pub fn waiting_at_most(mut self, patience: Duration) -> Self {
        self.patience = patience;
        self
    }

    /// One protocol message as a text frame.
    pub fn send<T: Serialize>(&self, frame: &T) {
        match serde_json::to_string(frame) {
            Ok(text) => self.put(Up::Text(text)),
            Err(error) => panic!("fake instance: a frame that does not serialise: {error}"),
        }
    }

    /// Text exactly as given, for the frames no protocol type can spell.
    pub fn send_text(&self, text: &str) {
        self.put(Up::Text(text.to_owned()));
    }

    /// A binary frame. It has no bytes because [`Received::Unsupported`]
    /// carries none.
    ///
    /// [`Received::Unsupported`]: crate::socket::port::Received::Unsupported
    pub fn send_binary(&self) {
        self.put(Up::Binary);
    }

    /// From here on the instance answers no ping, as a desktop that went to
    /// sleep with its connection half-open does not.
    pub fn go_silent(&self) {
        self.answering.store(false, Ordering::SeqCst);
    }

    /// From here on the instance reads nothing while its connection stays
    /// up: every write of the Hub end, a ping and a close among them, waits
    /// for good, as one does once the kernel's buffer to a peer that stopped
    /// reading is full. What the instance sends still arrives.
    pub fn stop_reading(&self) {
        self.reading.store(false, Ordering::SeqCst);
    }

    /// From here on every write of the Hub end, a ping and a close among
    /// them, fails with [`PeerGone`] and lands nowhere, as one does on a
    /// connection that broke without the read side having noticed yet.
    ///
    /// [`PeerGone`]: crate::socket::port::PeerGone
    pub fn refuse_sends(&self) {
        self.refusing.store(true, Ordering::SeqCst);
    }

    pub async fn expect_ack(&mut self, offset: u64) {
        let expected = HubFrame::Ack(CursorAck {
            v: PROTOCOL_VERSION,
            offset,
        });
        let sent = self.next(&expected).await;
        if sent != expected {
            panic!("fake instance: expected {expected}, the Hub sent {sent}");
        }
    }

    pub async fn expect_request(&mut self) -> SignedRequest {
        let expected = "a signed request";
        match self.next(&expected).await {
            HubFrame::Request(request) => *request,
            sent => panic!("fake instance: expected {expected}, the Hub sent {sent}"),
        }
    }

    pub async fn expect_close(&mut self, code: u16) {
        let expected = HubFrame::Close(code);
        let sent = self.next(&expected).await;
        if sent != expected {
            panic!("fake instance: expected {expected}, the Hub sent {sent}");
        }
    }

    /// Closes from the instance's side, waits for the Hub end to go away, and
    /// refuses every frame the script left unread, whenever it was sent.
    pub async fn finish(mut self) {
        self.close();
        let mut unread = Vec::new();
        let hub_gone = timeout(self.patience, async {
            while let Some(frame) = self.down.recv().await {
                unread.push(HubFrame::from(frame));
            }
        })
        .await;
        refuse_unread(&unread);
        if hub_gone.is_err() {
            panic!(
                "fake instance: the Hub end was still open {:?} after the instance closed",
                self.patience
            );
        }
    }

    /// A send to a Hub end that has gone is not a failure of the script. An
    /// instance cannot know its peer has just closed, and a script that sends
    /// two frames to be refused on the first would otherwise fail or pass by
    /// which side ran first. What the Hub did about it is asserted by the
    /// `expect_*` that follows.
    fn put(&self, frame: Up) {
        if let Some(up) = &self.up {
            up.send(frame).ok();
        }
    }

    fn close(&mut self) {
        self.up = None;
        self.answering.store(false, Ordering::SeqCst);
    }

    /// `expected` is `Sync` so that a script's future is `Send`: the
    /// Postgres-backed tests in `crates/demeteo-hub/tests/socket.rs` run each
    /// script inside a spawned task.
    async fn next(&mut self, expected: &(dyn fmt::Display + Sync)) -> HubFrame {
        match timeout(self.patience, self.down.recv()).await {
            Ok(Some(frame)) => HubFrame::from(frame),
            Ok(None) => panic!(
                "fake instance: expected {expected}, and the Hub end went away without sending it"
            ),
            Err(_) => panic!(
                "fake instance: expected {expected}, and the Hub sent nothing within {:?}",
                self.patience
            ),
        }
    }
}

/// The check of [`FakeInstance::finish`] over what has arrived so far: a drop
/// cannot wait for the Hub end. It is skipped while the thread unwinds, where
/// a second panic would abort the test binary and hide the first.
impl Drop for FakeInstance {
    fn drop(&mut self) {
        self.close();
        if std::thread::panicking() {
            return;
        }
        let mut unread = Vec::new();
        while let Ok(frame) = self.down.try_recv() {
            unread.push(HubFrame::from(frame));
        }
        refuse_unread(&unread);
    }
}

fn refuse_unread(unread: &[HubFrame]) {
    let Some(first) = unread.first() else {
        return;
    };
    let frames = match unread.len() {
        1 => "frame",
        _ => "frames",
    };
    panic!(
        "fake instance: the script ended with {} Hub {frames} unread, the first {first}",
        unread.len()
    );
}
