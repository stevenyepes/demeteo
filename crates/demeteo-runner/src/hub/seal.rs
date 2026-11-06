//! `demeteo-runner hub seal`: the key the runner's Hub secrets are sealed
//! under (decision 64). Thirty-two random bytes go from this process's memory
//! to `systemd-creds encrypt` on its stdin, and only the ciphertext reaches
//! disk; a unit drop-in then loads it with `LoadCredentialEncrypted=`, so
//! `serve` finds it in `$CREDENTIALS_DIRECTORY`.
//!
//! The credential is user-scoped (`--user`) because the unit is a
//! `systemd --user` one, and a user manager can only decrypt that scope. It
//! needs systemd 256 or later; a host whose `systemd-creds` cannot seal one
//! is refused, and does not pair.
//!
//! Which paths, arguments and plan are pure functions here; [`run`] is the
//! only I/O.
//!
//! It never restarts the service. A restart interrupts the runs in flight, and
//! the installers restart it anyway; run by hand, it says to.

use demeteo_core::adapters::database::SqliteAdapter;
use demeteo_core::adapters::hub::sealed_store::generate_store_key;
use demeteo_core::domain::hub_pairing::keyring_names::hub_keyring_account;
use demeteo_core::domain::hub_pairing::sealed::{STORE_KEY_CREDENTIAL, STORE_KEY_LEN};
use demeteo_core::domain::hub_pairing::HubSecret;
use demeteo_core::ports::hub::{HubPinRepository, HubSealedSecretRepository};
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const UNIT_NAME: &str = "demeteo-runner";
const CREDENTIAL_FILE: &str = "hub-store-key.cred";
const DROP_IN_FILE: &str = "hub-credential.conf";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealPaths {
    /// In the runner's `0700` data dir, beside the database it unlocks.
    pub credential: PathBuf,
    pub drop_in: PathBuf,
}

/// Where the sealed key and the drop-in go. The drop-in sits beside the unit
/// the installers write, under the user's systemd config directory.
pub fn seal_paths(
    data_dir: &Path,
    xdg_config_home: Option<&OsStr>,
    home: Option<&OsStr>,
) -> Result<SealPaths, String> {
    let config = match (xdg_config_home, home) {
        (Some(xdg), _) if Path::new(xdg).is_absolute() => PathBuf::from(xdg),
        (_, Some(home)) if Path::new(home).is_absolute() => Path::new(home).join(".config"),
        _ => return Err("neither XDG_CONFIG_HOME nor HOME is an absolute path".to_owned()),
    };
    Ok(SealPaths {
        credential: data_dir.join(CREDENTIAL_FILE),
        drop_in: config
            .join("systemd")
            .join("user")
            .join(format!("{UNIT_NAME}.service.d"))
            .join(DROP_IN_FILE),
    })
}

/// The drop-in loading `credential` as `hub-store-key`. systemd expands `%`
/// in the path, so it is doubled; a path systemd would read differently than
/// written is refused instead.
pub fn drop_in_contents(credential: &Path) -> Result<String, String> {
    let path = credential
        .to_str()
        .filter(|p| credential.is_absolute() && !p.chars().any(char::is_control))
        .ok_or_else(|| {
            format!(
                "{} cannot be named in a unit file: it must be an absolute UTF-8 path with \
                 no control characters",
                credential.display()
            )
        })?;
    Ok(format!(
        "# Written by `demeteo-runner hub seal`: the key the runner's Hub\n\
         # secrets are sealed under (docs/HUB.md §4).\n\
         [Service]\n\
         LoadCredentialEncrypted={STORE_KEY_CREDENTIAL}:{}\n",
        path.replace('%', "%%")
    ))
}

/// `systemd-creds` reading the key from stdin and writing the ciphertext to
/// `output`. `--name` is what the unit's credential is called, which
/// `systemd-creds` binds into the ciphertext.
pub fn encrypt_args(output: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["encrypt", "--user", &name_arg(), "-"]
        .iter()
        .map(OsString::from)
        .collect();
    args.push(output.into());
    args
}

/// `systemd-creds` decrypting `input` to stdout, so the round trip is checked
/// without the plaintext touching disk.
pub fn decrypt_args(input: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["decrypt", "--user", &name_arg()]
        .iter()
        .map(OsString::from)
        .collect();
    args.push(input.into());
    args.push("-".into());
    args
}

fn name_arg() -> String {
    format!("--name={STORE_KEY_CREDENTIAL}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SealPlan {
    /// A key is sealed already: only the drop-in is rewritten. This is what
    /// makes the installers' run of `hub seal` safe on an upgrade.
    KeepExisting,
    /// Seal a new key. Whatever the old key sealed is deleted, since nothing
    /// can open it; with `forget_pairing` the pins go too.
    Seal {
        forget_pairing: bool,
    },
    Refuse(String),
}

/// A new key orphans every secret the old one sealed, so a paired runner is
/// only re-sealed on `--force`, which forgets the pairing with it.
pub fn seal_plan(credential_exists: bool, paired: bool, force: bool) -> SealPlan {
    match (credential_exists, paired, force) {
        (true, _, false) => SealPlan::KeepExisting,
        (false, true, false) => SealPlan::Refuse(
            "this runner is paired but its sealed key is missing, so its Hub secrets cannot \
             be opened; `demeteo-runner hub seal --force` seals a new key and forgets the \
             pairing, after which `demeteo-runner hub pair <url>` pairs it again"
                .to_owned(),
        ),
        (_, paired, _) => SealPlan::Seal {
            forget_pairing: paired,
        },
    }
}

/// Every child this spawns, with the environment every agent spawn gets.
fn command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    demeteo_core::shared::proc::sanitize_child_env(&mut cmd);
    cmd
}

fn failure(what: &str, output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    format!("{what} exited {}: {}", output.status, stderr.trim())
}

fn encrypt(key: &[u8; STORE_KEY_LEN], output: &Path) -> Result<(), String> {
    let mut child = command("systemd-creds")
        .args(encrypt_args(output))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run systemd-creds: {e}"))?;
    let written = child
        .stdin
        .take()
        .ok_or_else(|| "systemd-creds has no stdin".to_owned())
        .and_then(|mut stdin| {
            stdin
                .write_all(key)
                .map_err(|e| format!("cannot hand systemd-creds the key: {e}"))
        });
    let output = child
        .wait_with_output()
        .map_err(|e| format!("systemd-creds did not finish: {e}"))?;
    written?;
    if output.status.success() {
        Ok(())
    } else {
        Err(failure("systemd-creds encrypt --user", &output))
    }
}

fn decrypts_to(key: &[u8; STORE_KEY_LEN], input: &Path) -> Result<(), String> {
    let output = command("systemd-creds")
        .args(decrypt_args(input))
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run systemd-creds: {e}"))?;
    if !output.status.success() {
        return Err(failure("systemd-creds decrypt --user", &output));
    }
    if output.stdout.as_slice() != key {
        return Err("the sealed key did not decrypt to the key sealed".to_owned());
    }
    Ok(())
}

fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(mode))
        .map_err(|e| format!("cannot chmod {}: {e}", path.display()))
}

/// The pins, and both sealed secrets under the old key, straight from the
/// database: a store without the key that sealed them cannot delete them.
fn forget_sealed(db: &SqliteAdapter, forget_pairing: bool) -> Result<(), String> {
    let install_id = HubPinRepository::install_id(db).map_err(|e| e.to_string())?;
    if forget_pairing {
        HubPinRepository::clear(db).map_err(|e| e.to_string())?;
    }
    for secret in [HubSecret::RefreshToken, HubSecret::DeviceKey] {
        HubSealedSecretRepository::delete_sealed(db, &hub_keyring_account(&install_id, secret))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn write_drop_in(paths: &SealPaths) -> Result<(), String> {
    let contents = drop_in_contents(&paths.credential)?;
    if let Some(dir) = paths.drop_in.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    std::fs::write(&paths.drop_in, contents)
        .map_err(|e| format!("cannot write {}: {e}", paths.drop_in.display()))
}

/// Best-effort: the drop-in is on disk either way, and the installers reload
/// after this returns. Without a user manager to reach (an SSH session on a
/// host with no lingering), there is nothing to reload yet.
fn daemon_reload() {
    let reloaded = command("systemctl")
        .args(["--user", "daemon-reload"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run systemctl: {e}"))
        .and_then(|output| {
            if output.status.success() {
                Ok(())
            } else {
                Err(failure("systemctl --user daemon-reload", &output))
            }
        });
    if let Err(e) = reloaded {
        eprintln!(
            "[demeteo-runner] warning: {e}; run `systemctl --user daemon-reload` before \
             restarting the runner"
        );
    }
}

/// What `run` was told to do once the plan is known.
pub struct SealRequest<'a> {
    pub data_dir: &'a Path,
    pub paths: SealPaths,
    pub force: bool,
}

/// What `run` did, for the CLI to print.
pub enum Sealed {
    Kept,
    New { forgot_pairing: bool },
}

/// Seals, or keeps, the key. `disconnect` is called first when a running
/// `serve` holds a pairing about to be forgotten, so its socket stops before
/// the rows it reads are deleted; it may fail (no `serve`), and the rows are
/// deleted either way.
pub async fn run<F, Fut>(request: SealRequest<'_>, disconnect: F) -> Result<Sealed, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let SealRequest {
        data_dir,
        paths,
        force,
    } = request;
    let conn = demeteo_core::db::init_db(data_dir.to_path_buf())
        .map_err(|e| format!("cannot open the runner database: {e}"))?;
    let db = SqliteAdapter::new(conn)?;
    let paired = HubPinRepository::load(&db)
        .map_err(|e| e.to_string())?
        .is_some();

    let forget_pairing = match seal_plan(paths.credential.exists(), paired, force) {
        SealPlan::Refuse(why) => return Err(why),
        SealPlan::KeepExisting => {
            write_drop_in(&paths)?;
            daemon_reload();
            return Ok(Sealed::Kept);
        }
        SealPlan::Seal { forget_pairing } => forget_pairing,
    };

    let key = generate_store_key()?;
    let staged = paths.credential.with_extension("cred.new");
    let sealed = encrypt(&key, &staged)
        .and_then(|()| set_mode(&staged, 0o600))
        .and_then(|()| decrypts_to(&key, &staged));
    if let Err(e) = sealed {
        let _ = std::fs::remove_file(&staged);
        return Err(format!(
            "this host cannot seal a Hub store key, so it cannot pair: {e}. A user-scoped \
             credential needs systemd 256 or later"
        ));
    }

    if forget_pairing {
        if let Err(e) = disconnect().await {
            eprintln!("[demeteo-runner] no running serve disconnected ({e}); forgetting the pairing in the database");
        }
    }
    forget_sealed(&db, forget_pairing)?;
    std::fs::rename(&staged, &paths.credential).map_err(|e| {
        format!(
            "cannot move the sealed key into {}: {e}",
            paths.credential.display()
        )
    })?;
    write_drop_in(&paths)?;
    daemon_reload();
    Ok(Sealed::New {
        forgot_pairing: forget_pairing,
    })
}

#[cfg(test)]
#[path = "seal_tests.rs"]
mod tests;
