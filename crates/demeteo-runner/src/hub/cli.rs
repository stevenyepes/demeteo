//! `demeteo-runner hub …`. Parsing and wording are pure; [`run`] talks to
//! `serve` over the control socket, except `seal`, which needs no `serve`.

use super::client::ControlClient;
use super::seal::{self, SealRequest, Sealed, UNIT_NAME};
use demeteo_core::domain::hub_request::HubLocalPolicy;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

pub const USAGE: &str = "\
demeteo-runner hub pair <hub-url>     pair this runner with a Demeteo Hub by device code
demeteo-runner hub status             show the pairing and the Hub connection
demeteo-runner hub policy [--read on|off] [--spend on|off] [--gates on|off]
                          [--run-ceiling <cents>|off] [--daily-ceiling <cents>|off]
                                      show, or change, what the Hub may do here
demeteo-runner hub disconnect         forget the Hub on this runner
demeteo-runner hub seal [--force]     seal the key the Hub secrets are stored under";

/// How often `hub pair` asks `serve` how the pairing stands. `serve` polls
/// the Hub on its own interval; this only decides how soon the CLI notices.
const PAIR_STATE_POLL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubCommand {
    Pair { hub_url: String },
    Status,
    Policy(PolicyPatch),
    Disconnect,
    Seal { force: bool },
}

/// The flags of `hub policy`. `None` leaves a field as stored; a ceiling of
/// `Some(None)` unsets it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PolicyPatch {
    pub read: Option<bool>,
    pub spend: Option<bool>,
    pub gates: Option<bool>,
    pub run_ceiling_cents: Option<Option<u64>>,
    pub daily_ceiling_cents: Option<Option<u64>>,
}

impl PolicyPatch {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn apply(&self, policy: HubLocalPolicy) -> HubLocalPolicy {
        HubLocalPolicy {
            read_enabled: self.read.unwrap_or(policy.read_enabled),
            spend_enabled: self.spend.unwrap_or(policy.spend_enabled),
            gates_enabled: self.gates.unwrap_or(policy.gates_enabled),
            run_ceiling_cents: self.run_ceiling_cents.unwrap_or(policy.run_ceiling_cents),
            daily_ceiling_cents: self
                .daily_ceiling_cents
                .unwrap_or(policy.daily_ceiling_cents),
        }
    }
}

/// `args` are what follows `hub`.
pub fn parse(args: &[String]) -> Result<HubCommand, String> {
    let (command, rest) = args
        .split_first()
        .ok_or_else(|| "missing a hub command".to_owned())?;
    match (command.as_str(), rest) {
        ("pair", [hub_url]) => Ok(HubCommand::Pair {
            hub_url: hub_url.clone(),
        }),
        ("pair", _) => Err("`hub pair` takes exactly one Hub URL".to_owned()),
        ("status", []) => Ok(HubCommand::Status),
        ("disconnect", []) => Ok(HubCommand::Disconnect),
        ("seal", []) => Ok(HubCommand::Seal { force: false }),
        ("seal", [flag]) if flag == "--force" => Ok(HubCommand::Seal { force: true }),
        ("policy", flags) => parse_policy(flags).map(HubCommand::Policy),
        (other, _) => Err(format!("unknown or malformed hub command: {other}")),
    }
}

fn parse_policy(flags: &[String]) -> Result<PolicyPatch, String> {
    let mut patch = PolicyPatch::default();
    let mut flags = flags.iter();
    while let Some(flag) = flags.next() {
        let value = flags
            .next()
            .ok_or_else(|| format!("{flag} needs a value"))?;
        let previous_was_set = match flag.as_str() {
            "--read" => patch.read.replace(switch(flag, value)?).is_some(),
            "--spend" => patch.spend.replace(switch(flag, value)?).is_some(),
            "--gates" => patch.gates.replace(switch(flag, value)?).is_some(),
            "--run-ceiling" => patch
                .run_ceiling_cents
                .replace(ceiling(flag, value)?)
                .is_some(),
            "--daily-ceiling" => patch
                .daily_ceiling_cents
                .replace(ceiling(flag, value)?)
                .is_some(),
            other => return Err(format!("unknown policy flag: {other}")),
        };
        if previous_was_set {
            return Err(format!("{flag} is given twice"));
        }
    }
    Ok(patch)
}

fn switch(flag: &str, value: &str) -> Result<bool, String> {
    match value {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(format!("{flag} takes on or off, not {value:?}")),
    }
}

/// Whole cents, or `off` to unset. An unset ceiling refuses every spend
/// (docs/HUB.md §7), so `off` is not "unlimited".
fn ceiling(flag: &str, value: &str) -> Result<Option<u64>, String> {
    match value {
        "off" => Ok(None),
        cents => cents
            .parse::<u64>()
            .map(Some)
            .map_err(|_| format!("{flag} takes whole cents or off, not {value:?}")),
    }
}

pub fn describe_policy(policy: &HubLocalPolicy) -> String {
    let switch = |on: bool| if on { "on" } else { "off" };
    let ceiling = |cents: Option<u64>| match cents {
        Some(c) => format!("{c} cents"),
        None => "unset (refuses every spend)".to_owned(),
    };
    format!(
        "read:          {}\nspend:         {}\ngates:         {}\nrun ceiling:   {}\ndaily ceiling: {}",
        switch(policy.read_enabled),
        switch(policy.spend_enabled),
        switch(policy.gates_enabled),
        ceiling(policy.run_ceiling_cents),
        ceiling(policy.daily_ceiling_cents),
    )
}

/// `DeviceTicket` as `hub_pair_begin` answers it.
#[derive(Debug, Deserialize)]
pub struct Ticket {
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub fingerprint: String,
    pub expires_at: u64,
}

pub fn describe_ticket(ticket: &Ticket, now_secs: u64) -> String {
    let minutes = ticket.expires_at.saturating_sub(now_secs).div_ceil(60);
    format!(
        "To pair this runner, open\n\n    {}\n\nor open {} and enter the code\n\n    {}\n\n\
         Approve it only if the Hub shows this runner's fingerprint:\n\n    {}\n\n\
         The code expires in {minutes} min. Waiting for approval (Ctrl-C cancels)…",
        ticket.verification_uri_complete,
        ticket.verification_uri,
        ticket.user_code,
        ticket.fingerprint,
    )
}

pub fn describe_status(view: &Value) -> String {
    let text = |v: &Value| match v {
        Value::Null => "—".to_owned(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let status = &view["status"];
    let mut lines = vec![
        format!("state:       {}", text(&status["state"])),
        format!("hub:         {}", text(&status["hub_url"])),
        format!("instance id: {}", text(&status["instance_id"])),
        format!("fingerprint: {}", text(&status["fingerprint"])),
        format!(
            "scopes:      {}",
            status["scopes"]
                .as_array()
                .filter(|s| !s.is_empty())
                .map(|s| s.iter().map(text).collect::<Vec<_>>().join(" "))
                .unwrap_or_else(|| "—".to_owned())
        ),
        format!(
            "socket:      {}{}",
            text(&view["connection"]["state"]),
            match &view["connection"]["stop_reason"] {
                Value::Null => String::new(),
                reason => format!(" ({})", text(reason)),
            }
        ),
        format!(
            "this runner: {} ({} {}, {})",
            text(&view["machine"]["instance_name"]),
            text(&view["machine"]["os"]),
            text(&view["machine"]["arch"]),
            text(&view["machine"]["app_version"]),
        ),
        format!("store:       {}", text(&status["keyring_backend"])),
    ];
    if let Some(why) = view["store_unavailable"].as_str() {
        lines.push(format!("cannot pair: {why}"));
    }
    lines.join("\n")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The exit code: `0` done, `1` refused or failed, `130` cancelled.
pub async fn run(command: HubCommand, data_dir: &Path) -> i32 {
    let socket = crate::rpc::socket_path(data_dir);
    let outcome = match command {
        HubCommand::Seal { force } => run_seal(data_dir, &socket, force).await,
        HubCommand::Pair { hub_url } => pair(&socket, &hub_url).await,
        HubCommand::Status => {
            with_client(&socket, |mut c| async move {
                let view = c.call("hub_status", json!({})).await?;
                Ok(describe_status(&view))
            })
            .await
        }
        HubCommand::Disconnect => {
            with_client(&socket, |mut c| async move {
                let status = c.call("hub_disconnect", json!({})).await?;
                Ok(format!(
                    "Disconnected. This runner is {} with the Hub.",
                    status["state"].as_str().unwrap_or("unknown")
                ))
            })
            .await
        }
        HubCommand::Policy(patch) => {
            with_client(&socket, |mut c| async move {
                let stored = policy(c.call("hub_policy_get", json!({})).await?)?;
                if patch.is_empty() {
                    return Ok(describe_policy(&stored));
                }
                let wanted = patch.apply(stored);
                let saved = policy(
                    c.call("hub_policy_set", json!({ "policy": wanted }))
                        .await?,
                )?;
                Ok(describe_policy(&saved))
            })
            .await
        }
    };
    match outcome {
        Ok(Done::Said(text)) => {
            println!("{text}");
            0
        }
        Ok(Done::Cancelled) => {
            eprintln!("Pairing cancelled.");
            130
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

enum Done {
    Said(String),
    Cancelled,
}

async fn with_client<F, Fut>(socket: &Path, f: F) -> Result<Done, String>
where
    F: FnOnce(ControlClient) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    f(ControlClient::connect(socket).await?)
        .await
        .map(Done::Said)
}

fn policy(value: Value) -> Result<HubLocalPolicy, String> {
    serde_json::from_value(value).map_err(|e| format!("the runner sent an unreadable policy: {e}"))
}

async fn pair(socket: &Path, hub_url: &str) -> Result<Done, String> {
    let mut client = ControlClient::connect(socket).await?;
    let ticket: Ticket = serde_json::from_value(
        client
            .call("hub_pair_begin", json!({ "hub_url": hub_url }))
            .await?,
    )
    .map_err(|e| format!("the runner sent an unreadable pairing code: {e}"))?;
    println!("{}", describe_ticket(&ticket, now_secs()));

    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    loop {
        tokio::select! {
            _ = &mut ctrl_c => {
                client.call("hub_pair_cancel", json!({})).await?;
                return Ok(Done::Cancelled);
            }
            () = tokio::time::sleep(PAIR_STATE_POLL) => {}
        }
        let state = client.call("hub_pair_state", json!({})).await?;
        match state["state"].as_str() {
            Some("pending") => continue,
            Some("failed") => {
                return Err(format!(
                    "the pairing failed: {}",
                    state["error"].as_str().unwrap_or("no reason given")
                ))
            }
            _ => break,
        }
    }
    let view = client.call("hub_status", json!({})).await?;
    match view["status"]["state"].as_str() {
        Some("paired") => Ok(Done::Said(format!(
            "Paired with {} as instance {}.",
            view["status"]["hub_url"].as_str().unwrap_or("the Hub"),
            view["status"]["instance_id"].as_str().unwrap_or("unknown"),
        ))),
        _ => Err("the pairing ended without pairing; was it cancelled from another client?".into()),
    }
}

async fn run_seal(data_dir: &Path, socket: &Path, force: bool) -> Result<Done, String> {
    let paths = seal::seal_paths(
        data_dir,
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )?;
    let restart = format!("systemctl --user restart {UNIT_NAME}");
    let credential = paths.credential.display().to_string();
    let drop_in = paths.drop_in.display().to_string();
    let disconnect = || async {
        let mut client = ControlClient::connect(socket).await?;
        client.call("hub_disconnect", json!({})).await.map(|_| ())
    };
    let sealed = seal::run(
        SealRequest {
            data_dir,
            paths,
            force,
        },
        disconnect,
    )
    .await?;
    Ok(Done::Said(match sealed {
        Sealed::Kept => format!(
            "A Hub store key is sealed already at {credential}; kept it (--force replaces \
             it). The unit loads it through {drop_in}."
        ),
        Sealed::New { forgot_pairing } => format!(
            "Sealed a new Hub store key at {credential}, loaded through {drop_in}.{}\n\
             Restart the runner to load it: {restart}",
            if forgot_pairing {
                " The previous pairing was forgotten; pair again with `demeteo-runner hub \
                 pair <url>`."
            } else {
                ""
            }
        ),
    }))
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
