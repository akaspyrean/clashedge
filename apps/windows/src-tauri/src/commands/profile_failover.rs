use tauri::{command, AppHandle, State};

use crate::core::profile_failover::ProfileFailoverStatus;
use crate::util::error::{Error, Result};

#[command]
pub fn get_profile_failover(
    state: State<'_, crate::AppState>,
) -> Result<ProfileFailoverStatus> {
    let manager = state
        .profile_failover
        .get()
        .ok_or_else(|| Error::InvalidState("profile failover is not initialized".to_string()))?;
    Ok(manager.status())
}

#[command]
pub fn set_backup_profile(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    backup_profile: Option<String>,
) -> Result<ProfileFailoverStatus> {
    let selected = if let Some(name) = backup_profile {
        let safe_name = crate::util::paths::sanitize_profile_name(&name)?;
        let profiles_dir = crate::util::paths::get_profiles_dir(&app)?;
        let path = profiles_dir.join(format!("{}.yaml", safe_name));
        if !path.is_file() {
            return Err(Error::NotFound("Backup profile not found".to_string()));
        }
        let active = state
            .config_manager
            .lock()
            .unwrap()
            .get_config()
            .general
            .profile;
        if active == safe_name {
            return Err(Error::InvalidArgument(
                "backup profile must differ from the active profile".to_string(),
            ));
        }
        Some(safe_name)
    } else {
        None
    };

    let manager = state
        .profile_failover
        .get()
        .ok_or_else(|| Error::InvalidState("profile failover is not initialized".to_string()))?;
    manager.set_backup_profile(selected)
}
