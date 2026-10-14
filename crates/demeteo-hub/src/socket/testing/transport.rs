//! The channel pair that stands where the WebSocket does. Why a write here
//! can wait for good or fail on purpose, and what of it is real time, is on
//! [the module](super).

use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::socket::port::{PeerGone, Received, Transport};

pub(super) enum Up {
    Text(String),
    Binary,
}

pub(super) enum Down {
    Text(String),
    Close(u16),
}

/// The Hub's end of the pair. A test that has no session yet drives it by
/// hand through [`Transport`].
pub struct ChannelTransport {
    pub(super) up: mpsc::UnboundedReceiver<Up>,
    pub(super) down: mpsc::UnboundedSender<Down>,
    pub(super) answering: Arc<AtomicBool>,
    pub(super) reading: Arc<AtomicBool>,
    pub(super) refusing: Arc<AtomicBool>,
    pub(super) pongs: usize,
}

impl ChannelTransport {
    /// What every write meets before it lands: `PeerGone` once the instance
    /// [refuses sends](super::FakeInstance::refuse_sends), a wait that never
    /// ends once it [stopped reading](super::FakeInstance::stop_reading), and
    /// otherwise nothing.
    async fn handed_over(&self) -> Result<(), PeerGone> {
        if self.refusing.load(Ordering::SeqCst) {
            return Err(PeerGone);
        }
        if !self.reading.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        Ok(())
    }
}

#[async_trait]
impl Transport for ChannelTransport {
    async fn received(&mut self) -> Received {
        if self.pongs > 0 {
            self.pongs -= 1;
            return Received::Alive;
        }
        match self.up.recv().await {
            Some(Up::Text(text)) => Received::Text(text),
            Some(Up::Binary) => Received::Unsupported,
            None => Received::Closed,
        }
    }

    /// Succeeds for as long as the fake exists, its own close included,
    /// unless the script said otherwise: a frame sent after the instance
    /// closed has to land where [`FakeInstance::finish`] will find it.
    ///
    /// [`FakeInstance::finish`]: super::FakeInstance::finish
    async fn send_text(&mut self, text: String) -> Result<(), PeerGone> {
        self.handed_over().await?;
        self.down.send(Down::Text(text)).map_err(|_| PeerGone)
    }

    async fn send_ping(&mut self) -> Result<(), PeerGone> {
        self.handed_over().await?;
        if self.down.is_closed() {
            return Err(PeerGone);
        }
        if self.answering.load(Ordering::SeqCst) {
            self.pongs += 1;
        }
        Ok(())
    }

    async fn close(self, code: u16) {
        if self.handed_over().await.is_ok() {
            self.down.send(Down::Close(code)).ok();
        }
    }
}
