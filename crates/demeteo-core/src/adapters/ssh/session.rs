//! Connection pooling for the SSH adapter: the per-machine SFTP/SSH session
//! cache, its liveness policy, and the credential lookup that opens a new
//! connection. Everything that answers "give me a live session for machine X"
//! lives here so the `ExecutionPort` impl in `client.rs` only deals with what
//! it does *over* that session.

use crate::domain::models::Machine;
use crate::ports::db::MachineRepository;
use ssh2::{Session, Sftp};
use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

pub struct SftpSession {
    pub sftp: Mutex<Sftp>,
    pub session: Session,
    pub tcp: TcpStream,
}

/// Look up the stored secret for `machine` in the OS keyring (via the
/// process-wide credential cache), which is the only place credentials live.
///
/// Only `password` and `key` machines have one; anything else (notably
/// `local`, and agent-forwarding setups) resolves to `None` by design. The
/// `keyring` cargo feature gates the actual keyring call — builds without it
/// take the `#[cfg(not(feature = "keyring"))]` arm and always error out of the
/// fetch. Either way a miss degrades to `None` rather than failing the caller:
/// authentication then proceeds without a secret (agent/pubkey-by-path), which
/// is what the three call sites this was extracted from all did.
pub(super) fn machine_secret(machine: &Machine) -> Option<String> {
    match machine.auth_type.as_str() {
        "password" | "key" => {
            let key = format!("machine_{}", machine.id);
            crate::credential_cache::get_or_fetch(&key, || {
                #[cfg(feature = "keyring")]
                {
                    let entry = keyring::Entry::new("demeteo", &key)
                        .map_err(|e| format!("Keyring error: {}", e))?;
                    entry
                        .get_password()
                        .map_err(|e| format!("Keyring error: {}", e))
                }
                #[cfg(not(feature = "keyring"))]
                {
                    Err("OS-keyring credential cache is disabled in this build".to_string())
                }
            })
            .ok()
        }
        _ => None,
    }
}

/// Owns every pooled SSH/SFTP connection plus the per-machine remote-HOME
/// cache.
///
/// These used to be two loose `Arc<Mutex<HashMap<..>>>` fields on
/// `SshClientAdapter`, threaded through free functions as parameters purely so
/// each could be cloned out of `self` and moved into a `spawn_blocking`
/// closure — the blocking `ssh2` API means every port method does that, and
/// `&self` can't cross the boundary. Folding them into one `Arc<SessionPool>`
/// keeps that property (the closures clone one handle instead of three) while
/// letting the pooling logic be methods on the state it operates on, so the
/// eviction rule and the liveness probe have exactly one home.
pub(super) struct SessionPool {
    machines: Arc<dyn MachineRepository>,
    sessions: Mutex<HashMap<String, Arc<SftpSession>>>,
    /// Resolved remote HOME per machine_id. The remote HOME is stable
    /// for the lifetime of the user's account, so we cache it after the
    /// first successful resolve to avoid an extra `echo $HOME` round-trip
    /// on every path computation. Cleared on `disconnect_all` (which
    /// isn't called today, but the cache is keyed by `machine_id` so
    /// reconnects naturally pick up the cached value).
    ///
    /// `pub(super)` because [`SessionPool::resolve_home`] — the only reader —
    /// lives in the sibling `home` module with the rest of the HOME concern.
    pub(super) home_cache: Mutex<HashMap<String, String>>,
}

impl SessionPool {
    pub(super) fn new(machines: Arc<dyn MachineRepository>) -> Self {
        Self {
            machines,
            sessions: Mutex::new(HashMap::new()),
            home_cache: Mutex::new(HashMap::new()),
        }
    }

    /// The machine repository, for callers that only need to resolve a
    /// `Machine` record and never open a connection (`test_connection`,
    /// `resolve_user`, `spawn_interactive`).
    pub(super) fn machines(&self) -> &dyn MachineRepository {
        &*self.machines
    }

    /// Return a live SFTP session for `machine_id`, reusing the pooled one if
    /// it still answers and connecting a fresh one otherwise.
    pub(super) fn get(&self, machine_id: &str) -> Result<Arc<SftpSession>, String> {
        // Take a cheap `Arc` clone of any pooled session *under* the lock, then
        // release it before the liveness probe. The probe (`readdir`) is a blocking
        // network round-trip; running it while holding the global `sessions` mutex
        // means one wedged connection blocks every other machine's SSH ops behind
        // it — a pipeline-wide stall (the "stopped at validate" hang). Off the lock,
        // a slow probe only delays the caller that owns that connection.
        let pooled = {
            let sessions = self
                .sessions
                .lock()
                .map_err(|_| "Failed to lock SFTP state".to_string())?;
            sessions.get(machine_id).cloned()
        };

        if let Some(s) = pooled {
            let alive = match s.sftp.lock() {
                Ok(sftp) => sftp.readdir(std::path::Path::new(".")).is_ok(),
                Err(_) => false,
            };
            if alive {
                return Ok(s);
            }
            // Wedged/dead — evict it so the next caller reconnects. Only remove the
            // entry if it's still the same `Arc` we probed: a concurrent caller may
            // have already reconnected and inserted a fresh session while our probe
            // was blocking, and we must not drop that one on the floor.
            if let Ok(mut sessions) = self.sessions.lock() {
                if sessions
                    .get(machine_id)
                    .is_some_and(|cur| Arc::ptr_eq(cur, &s))
                {
                    sessions.remove(machine_id);
                }
            }
        }

        // Connect new session
        let machine = crate::infrastructure::worktree::machine_resolver::resolve_machine(
            self.machines(),
            machine_id,
        )?;

        let secret = machine_secret(&machine);

        let (sess, tcp) = crate::ssh_util::connect(&machine, secret)?;

        sess.set_blocking(true);
        let sftp = sess
            .sftp()
            .map_err(|e| format!("SFTP subsystem failed: {}", e))?;

        let sftp_session = Arc::new(SftpSession {
            sftp: Mutex::new(sftp),
            session: sess,
            tcp,
        });

        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "Failed to lock SFTP state".to_string())?;
        sessions.insert(machine_id.to_string(), sftp_session.clone());
        Ok(sftp_session)
    }

    /// Drop the pooled session for `machine_id` so the next [`SessionPool::get`]
    /// reconnects. Used by the SFTP operation paths, where a failed
    /// open/create/stat/readdir means the connection is suspect even though the
    /// liveness probe passed moments earlier. A poisoned lock is ignored — the
    /// error being reported to the caller matters more than the eviction.
    pub(super) fn evict(&self, machine_id: &str) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(machine_id);
        }
    }
}
