//! Which instances hold a socket to this process, and the two things said to
//! the session behind each: that its queue may have grown, and that it is to
//! end.
//!
//! Neither says anything more. What a session sends is read from the store
//! when it is woken, and what it writes on leaving is its own to write.

use super::MAX_CONNECTIONS;
use crate::store::InstanceId;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::sync::{watch, Notify};
use uuid::Uuid;

/// One registration, and not the instance it was made for: a session that
/// was displaced still leaves, and it names itself by this so that leaving
/// does not take its successor's entry with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionId(Uuid);

impl ConnectionId {
    /// What `instances.connection_id` holds while this registration is the
    /// one its instance is present through. One-way: an id read from that
    /// column is compared with this and never turned into a registration.
    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

/// Why a session is asked to end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Closing {
    /// A later connection of the same instance registered.
    Displaced,
    ShuttingDown,
}

/// [`MAX_CONNECTIONS`] instances are connected and this one is not among
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("too many instances are connected")]
pub struct Full;

/// The hint that this connection's instance may have requests queued.
#[derive(Debug)]
pub struct Wake(Arc<Notify>);

impl Wake {
    /// Returns once [`Connections::wake`] was called for this connection
    /// since the last time this returned, whether that was before the wait
    /// began or during it. Any number of calls in between are one wake-up:
    /// the caller reads the whole queue, so a count would say nothing.
    ///
    /// Dropped before it returns, it keeps a wake-up it had not handed over
    /// for the next wait.
    pub async fn woken(&self) {
        self.0.notified().await;
    }
}

/// What this connection was asked to end for, once it has been.
#[derive(Debug)]
pub struct Close(watch::Receiver<Option<Closing>>);

impl Close {
    /// Returns once this connection is asked to end, and the same answer
    /// every time after that.
    ///
    /// A connection the registry no longer holds, and asked nothing of
    /// while it did, answers [`Closing::ShuttingDown`]: one that was
    /// released, or whose registry is gone. Nothing can wake or displace it
    /// any more, and a session left waiting on an answer that cannot come
    /// would hold [`Connections::drained`] to its bound for nothing.
    pub async fn closing(&mut self) -> Closing {
        loop {
            let asked = *self.0.borrow_and_update();
            if let Some(closing) = asked {
                return closing;
            }
            if self.0.changed().await.is_err() {
                return Closing::ShuttingDown;
            }
        }
    }
}

/// What a session holds of its registration.
#[derive(Debug)]
pub struct Registered {
    pub connection: ConnectionId,
    pub wake: Wake,
    pub close: Close,
    pub(super) counted: Counted,
}

/// Keeps its registration among those [`Connections::drained`] waits for,
/// until it is dropped. Never read: holding it is all it does.
#[derive(Debug)]
pub(super) struct Counted {
    _alive: watch::Receiver<()>,
}

struct Live {
    connection: ConnectionId,
    wake: Arc<Notify>,
    close: watch::Sender<Option<Closing>>,
}

#[derive(Default)]
struct Registry {
    live: HashMap<InstanceId, Live>,
    shutting_down: bool,
}

/// Kept in process memory and nowhere else, as
/// [`Pairings`](crate::pairing::pending::Pairings) is, and at the two costs
/// stated there:
///
/// - **Lost on restart.** So is every socket it describes, and an instance
///   reconnects to a registry that holds only what registered since. What
///   was queued for it is in the store and is read by its next session.
/// - **Invisible to a second `hub` replica.** A request enqueued on one
///   replica wakes no session held by another, and waits in the store until
///   that instance reconnects. Displacement does not cross replicas either:
///   each can hold a connection of the same instance, and neither is told.
///
/// Where this differs from that store:
///
/// - **One entry per instance, and the latest registration holds it.** An
///   instance that reconnects has usually lost a socket the Hub has not yet
///   noticed is dead, so the newcomer is the live one. The earlier session is
///   told [`Closing::Displaced`] and not left to its ping deadline.
/// - **Nothing expires.** An entry is there until its session leaves, a later
///   connection takes its place, or the process ends.
/// - **A full registry refuses an instance that is not connected, and closes
///   no one to seat it.** Any entry it could evict is a working session of
///   some tenant, and the one refused can ask again. An instance that is
///   already connected is still let in at the bound: it takes its own place.
///
/// **Rejected: handing the request to the session through its entry.** A
/// channel of requests would be a second queue beside the rows, and the two
/// disagree the first time a session ends with one unread. A wake-up carries
/// nothing, so there is nothing in it to lose.
///
/// **What a stop waits for is every registration, not every entry.** A
/// session that was displaced has lost its entry and is still writing its
/// departure, so [`drained`](Self::drained) counts the [`Registered`] values
/// that are alive, outside the lock, and not the map.
///
/// Every method but [`drained`](Self::drained) is synchronous and returns
/// owned values, so the lock is released before a caller can reach an
/// `.await`. That one never takes the lock.
#[derive(Default)]
pub struct Connections {
    held: Mutex<Registry>,
    counted: watch::Sender<()>,
}

impl Connections {
    /// Seats a new connection of `instance` under a fresh id (a version 4
    /// UUID), and asks the one it replaces, if any, to end as
    /// [`Closing::Displaced`].
    ///
    /// After [`close_all`](Self::close_all) the connection is seated all the
    /// same and is already asked to end: an upgrade that was being admitted
    /// when the signal came would otherwise start a session nothing closes.
    pub fn register(&self, instance: InstanceId) -> Result<Registered, Full> {
        let connection = ConnectionId(Uuid::new_v4());
        let wake = Arc::new(Notify::new());

        let mut registry = self.lock();
        let asked = registry.shutting_down.then_some(Closing::ShuttingDown);
        let (close, closing) = watch::channel(asked);
        let live = Live {
            connection,
            wake: Arc::clone(&wake),
            close,
        };
        let full = registry.live.len() >= MAX_CONNECTIONS;
        match registry.live.entry(instance) {
            Entry::Occupied(mut held) => {
                let displaced = held.insert(live);
                displaced.close.send_replace(Some(Closing::Displaced));
            }
            Entry::Vacant(_) if full => return Err(Full),
            Entry::Vacant(place) => {
                place.insert(live);
            }
        }
        Ok(Registered {
            connection,
            wake: Wake(wake),
            close: Close(closing),
            counted: Counted {
                _alive: self.counted.subscribe(),
            },
        })
    }

    /// Returns once no [`Registered`] this registry handed out is alive,
    /// at once if none is. One made while this waits is waited for too.
    pub async fn drained(&self) {
        self.counted.closed().await;
    }

    /// How many [`Registered`] this registry handed out are alive, displaced
    /// ones included.
    pub fn alive(&self) -> usize {
        self.counted.receiver_count()
    }

    /// Tells the session of `instance`, if one is registered, to read its
    /// queue again. Call it after the write it is about has committed.
    ///
    /// `notify_one` and not `notify_waiters`: it keeps a permit when nobody
    /// is waiting. A session reads its queue and only then waits, and a
    /// wake-up sent between the two would otherwise be dropped, leaving the
    /// request in the store until the instance reconnects.
    pub fn wake(&self, instance: InstanceId) {
        if let Some(live) = self.lock().live.get(&instance) {
            live.wake.notify_one();
        }
    }

    /// Removes the entry of `instance` if `connection` still holds it. A
    /// displaced session calls this as any other does, and finds its
    /// successor there.
    pub fn release(&self, instance: InstanceId, connection: ConnectionId) {
        if let Entry::Occupied(held) = self.lock().live.entry(instance) {
            if held.get().connection == connection {
                held.remove();
            }
        }
    }

    /// Asks every registered connection to end as [`Closing::ShuttingDown`],
    /// and every one registered from now on. The entries stay: each session
    /// removes its own as it leaves.
    pub fn close_all(&self) {
        let mut registry = self.lock();
        registry.shutting_down = true;
        for live in registry.live.values() {
            live.close.send_replace(Some(Closing::ShuttingDown));
        }
    }

    /// Poisoning is answered as `Ceremonies` answers it, for the reason given
    /// on its `lock`. Signals are sent under this one as well as `HashMap`
    /// calls, and neither kind blocks or leaves an entry half-written.
    ///
    /// `send_replace` and never `send` on a close signal: `send` stores
    /// nothing when no receiver is left, and a session that dropped its
    /// [`Close`] is not an error here.
    fn lock(&self) -> MutexGuard<'_, Registry> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
