//! The production [`HubRunKeys`]: each reads what its instance logs runs
//! under and hands it to [`reported_feature_id`], which decides. Which one
//! an instance gets is chosen by its execution mode at composition.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use crate::domain::hub_socket::{reported_feature_id, EventOwner};
use crate::domain::ids::FeatureId;
use crate::ports::db::{FeatureRepository, MachineHubStateRepository};
use crate::ports::hub::{HubPinRepository, HubRunKeys};
use crate::ports::remote_run_mirror::RemoteRunMirrorPort;
use crate::ports::runner_run::RunnerRunPort;

/// A desktop logs every run under its Feature id, a shadow's included. A key
/// is a shadow when a mirrored remote run names it, and is judged by what
/// the newest such run's machine last said of its pairing; any other key is
/// the desktop's own, a Feature since deleted included, since the run log
/// still owes the Hub how it ended.
pub struct DesktopRunKeys {
    mirror: Arc<dyn RemoteRunMirrorPort>,
    machines: Arc<dyn MachineHubStateRepository>,
    pins: Arc<dyn HubPinRepository>,
}

impl DesktopRunKeys {
    pub fn new(
        mirror: Arc<dyn RemoteRunMirrorPort>,
        machines: Arc<dyn MachineHubStateRepository>,
        pins: Arc<dyn HubPinRepository>,
    ) -> Self {
        Self {
            mirror,
            machines,
            pins,
        }
    }
}

impl HubRunKeys for DesktopRunKeys {
    fn reported_ids(&self, keys: &BTreeSet<String>) -> Result<HashMap<String, String>, String> {
        let own = self
            .pins
            .load()
            .map_err(|e| e.to_string())?
            .map(|pins| pins.hub_origin);
        let states = self.machines.all()?;
        let mut newest: HashMap<String, (i64, String)> = HashMap::new();
        for run in self.mirror.list()? {
            let Some(feature_id) = run.feature_id.filter(|id| keys.contains(id)) else {
                continue;
            };
            match newest.get(&feature_id) {
                Some((created_at, _)) if *created_at >= run.created_at => {}
                _ => {
                    newest.insert(feature_id, (run.created_at, run.machine_id));
                }
            }
        }
        Ok(keys
            .iter()
            .filter_map(|key| {
                let owner = match newest.get(key) {
                    Some((_, machine_id)) => EventOwner::Mirrored {
                        runner: states.get(machine_id),
                    },
                    None => EventOwner::Local,
                };
                Some((
                    key.clone(),
                    reported_feature_id(key, owner, own.as_deref())?,
                ))
            })
            .collect())
    }
}

/// A runner logs a run's events under its run id, and its snapshot lists
/// the Feature the run bootstrapped, whose id is the one the submitting
/// desktop chose for its shadow. A key that is a Feature id is reported as
/// itself; one that names neither a run nor a Feature is not reported.
pub struct RunnerRunKeys {
    runs: Arc<dyn RunnerRunPort>,
    features: Arc<dyn FeatureRepository>,
}

impl RunnerRunKeys {
    pub fn new(runs: Arc<dyn RunnerRunPort>, features: Arc<dyn FeatureRepository>) -> Self {
        Self { runs, features }
    }
}

impl HubRunKeys for RunnerRunKeys {
    fn reported_ids(&self, keys: &BTreeSet<String>) -> Result<HashMap<String, String>, String> {
        let mut out = HashMap::new();
        for key in keys {
            let run = self.runs.get(key)?;
            let owner = match &run {
                Some(run) => EventOwner::RunnerRun {
                    feature_id: run.feature_id.as_deref(),
                },
                None if self.features.get(&FeatureId::from(key.clone()))?.is_some() => {
                    EventOwner::Local
                }
                None => EventOwner::Unknown,
            };
            if let Some(id) = reported_feature_id(key, owner, None) {
                out.insert(key.clone(), id);
            }
        }
        Ok(out)
    }
}

/// Every key reported as itself: what a socket reports when its composition
/// names no other.
pub struct ReportedAsIs;

impl HubRunKeys for ReportedAsIs {
    fn reported_ids(&self, keys: &BTreeSet<String>) -> Result<HashMap<String, String>, String> {
        Ok(keys.iter().map(|key| (key.clone(), key.clone())).collect())
    }
}
