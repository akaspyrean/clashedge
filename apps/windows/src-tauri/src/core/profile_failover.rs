//! Optional, user-selected backup profile and persisted patrol state.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::util::error::{Error, Result};

const STATE_FILE: &str = "profile-failover.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveFailover {
    pub primary_profile: String,
    pub backup_profile: String,
    pub started_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct StoredState {
    #[serde(default)]
    backup_profile: Option<String>,
    #[serde(default)]
    active: Option<ActiveFailover>,
    #[serde(default)]
    manual_override_profile: Option<String>,
    #[serde(default)]
    probe_in_progress: bool,
    #[serde(default)]
    pending_manual_profile: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProfileFailoverStatus {
    pub backup_profile: Option<String>,
    pub active: Option<ActiveFailover>,
}

pub struct ProfileFailoverManager {
    path: PathBuf,
    state: Mutex<StoredState>,
}

impl ProfileFailoverManager {
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join(STATE_FILE);
        let state = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                warn!("Ignoring invalid profile failover state: {}", error);
                StoredState::default()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => StoredState::default(),
            Err(error) => {
                warn!("Unable to read profile failover state: {}", error);
                StoredState::default()
            }
        };
        Self {
            path,
            state: Mutex::new(state),
        }
    }

    pub fn status(&self) -> ProfileFailoverStatus {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        ProfileFailoverStatus {
            backup_profile: state.backup_profile.clone(),
            active: state.active.clone(),
        }
    }

    pub fn active(&self) -> Option<ActiveFailover> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .active
            .clone()
    }

    pub fn backup_profile(&self) -> Option<String> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .backup_profile
            .clone()
    }

    pub fn set_backup_profile(&self, profile: Option<String>) -> Result<ProfileFailoverStatus> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.active.is_some() {
            return Err(Error::InvalidState(
                "cannot change the backup profile while failover is active".to_string(),
            ));
        }
        let previous = state.clone();
        state.backup_profile = profile.filter(|name| !name.trim().is_empty());
        state.manual_override_profile = None;
        state.probe_in_progress = false;
        state.pending_manual_profile = None;
        if let Err(error) = self.persist(&state) {
            *state = previous;
            return Err(error);
        }
        Ok(ProfileFailoverStatus {
            backup_profile: state.backup_profile.clone(),
            active: state.active.clone(),
        })
    }

    pub fn begin(&self, primary_profile: &str, backup_profile: &str) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.active.is_some()
            || state.pending_manual_profile.is_some()
            || state.backup_profile.as_deref() != Some(backup_profile)
            || state.manual_override_profile.as_deref() == Some(primary_profile)
        {
            return Err(Error::InvalidState(
                "profile failover settings changed before activation".to_string(),
            ));
        }
        let previous = state.clone();
        state.probe_in_progress = false;
        state.active = Some(ActiveFailover {
            primary_profile: primary_profile.to_string(),
            backup_profile: backup_profile.to_string(),
            started_at_ms: now_ms(),
        });
        if let Err(error) = self.persist(&state) {
            *state = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn clear_active(&self) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.active = None;
        state.probe_in_progress = false;
        self.persist(&state)
    }

    pub fn probe_in_progress(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .probe_in_progress
    }

    pub fn set_probe_in_progress(&self, in_progress: bool) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.active.is_none() {
            return Err(Error::InvalidState(
                "no active profile failover to probe".to_string(),
            ));
        }
        let previous = state.probe_in_progress;
        state.probe_in_progress = in_progress;
        if let Err(error) = self.persist(&state) {
            state.probe_in_progress = previous;
            return Err(error);
        }
        Ok(())
    }

    /// Persist manual intent before an activation that would supersede failover.
    /// If this write fails, the profile activation must not proceed.
    pub fn begin_manual_profile_activation(&self, profile: &str) -> Result<bool> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let needed = state
            .active
            .as_ref()
            .is_some_and(|active| active.backup_profile.as_str() != profile)
            || state
                .manual_override_profile
                .as_deref()
                .is_some_and(|override_profile| override_profile != profile);
        if !needed {
            return Ok(false);
        }
        let previous = state.pending_manual_profile.clone();
        state.pending_manual_profile = Some(profile.to_string());
        if let Err(error) = self.persist(&state) {
            state.pending_manual_profile = previous;
            return Err(error);
        }
        Ok(true)
    }

    pub fn cancel_manual_profile_activation(&self, profile: &str) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.pending_manual_profile.as_deref() != Some(profile) {
            return Ok(());
        }
        state.pending_manual_profile = None;
        if let Err(error) = self.persist(&state) {
            state.pending_manual_profile = Some(profile.to_string());
            return Err(error);
        }
        Ok(())
    }

    /// Reconcile a previously persisted intent after activation or process restart.
    /// A matching active profile is manual; a mismatch means activation never committed.
    pub fn reconcile_pending_manual_activation(&self, profile: &str) -> Result<bool> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(pending_profile) = state.pending_manual_profile.clone() else {
            return Ok(false);
        };
        let applied = pending_profile == profile;
        state.pending_manual_profile = None;
        if applied {
            state.active = None;
            state.probe_in_progress = false;
            state.manual_override_profile = Some(profile.to_string());
            // The on-disk pending intent still protects this profile on restart.
            self.persist(&state)?;
        } else if let Err(error) = self.persist(&state) {
            state.pending_manual_profile = Some(pending_profile);
            return Err(error);
        }
        Ok(applied)
    }

    pub fn dismiss_active_for_manual_change(&self, profile: &str) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = state.clone();
        state.pending_manual_profile = Some(profile.to_string());
        if let Err(error) = self.persist(&state) {
            *state = previous;
            return Err(error);
        }
        state.active = None;
        state.probe_in_progress = false;
        state.manual_override_profile = Some(profile.to_string());
        state.pending_manual_profile = None;
        self.persist(&state)
    }

    /// Returns true while this profile remains the user's manual override.
    /// A later explicit profile change clears the suppression.
    pub fn is_manually_overridden(&self, profile: &str) -> Result<bool> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match state.manual_override_profile.as_deref() {
            Some(override_profile) if override_profile == profile => Ok(true),
            Some(_) => {
                let previous = state.manual_override_profile.clone();
                state.manual_override_profile = None;
                if let Err(error) = self.persist(&state) {
                    state.manual_override_profile = previous;
                    return Err(error);
                }
                Ok(false)
            }
            None => Ok(false),
        }
    }

    pub fn note_manual_profile_activation(&self, profile: &str) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let should_dismiss = state
            .active
            .as_ref()
            .is_some_and(|active| active.backup_profile.as_str() != profile);
        let should_clear_override = state
            .manual_override_profile
            .as_deref()
            .is_some_and(|override_profile| override_profile != profile);
        let should_clear_pending = state.pending_manual_profile.as_deref() == Some(profile);
        if !should_dismiss && !should_clear_override && !should_clear_pending {
            return Ok(());
        }
        if should_dismiss {
            state.active = None;
            state.probe_in_progress = false;
            state.manual_override_profile = Some(profile.to_string());
        } else if should_clear_override {
            state.manual_override_profile = None;
        }
        state.pending_manual_profile = None;
        self.persist(&state)?;
        Ok(())
    }

    pub fn rename_profile(&self, old_name: &str, new_name: &str) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.backup_profile.as_deref() == Some(old_name) {
            state.backup_profile = Some(new_name.to_string());
        }
        if let Some(active) = state.active.as_mut() {
            if active.primary_profile.as_str() == old_name {
                active.primary_profile = new_name.to_string();
            }
            if active.backup_profile.as_str() == old_name {
                active.backup_profile = new_name.to_string();
            }
        }
        if state.manual_override_profile.as_deref() == Some(old_name) {
            state.manual_override_profile = Some(new_name.to_string());
        }
        if state.pending_manual_profile.as_deref() == Some(old_name) {
            state.pending_manual_profile = Some(new_name.to_string());
        }
        self.persist(&state)?;
        Ok(())
    }

    pub fn delete_profile(&self, name: &str) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.backup_profile.as_deref() == Some(name) {
            state.backup_profile = None;
        }
        if state.active.as_ref().is_some_and(|active| {
            active.primary_profile.as_str() == name || active.backup_profile.as_str() == name
        }) {
            state.active = None;
            state.probe_in_progress = false;
        }
        if state.manual_override_profile.as_deref() == Some(name) {
            state.manual_override_profile = None;
        }
        if state.pending_manual_profile.as_deref() == Some(name) {
            state.pending_manual_profile = None;
        }
        self.persist(&state)?;
        Ok(())
    }

    fn persist(&self, state: &StoredState) -> Result<()> {
        let bytes = serde_json::to_vec(state).map_err(|error| {
            Error::Other(format!("serialize profile failover state: {}", error))
        })?;
        crate::util::atomic::atomic_write(&self.path, &bytes)
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}
