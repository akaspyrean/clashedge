use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

static SENT_KEYS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

/// Show a native system notification. Callers should log failures without failing
/// the subscription refresh or core-recovery operation that triggered it.
pub fn send(app: &AppHandle, title: &str, body: &str) -> std::result::Result<(), String> {
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|error| error.to_string())
}

/// Deduplicate repeated threshold notices within one application session.
pub fn send_once(
    app: &AppHandle,
    key: String,
    title: &str,
    body: &str,
) -> std::result::Result<(), String> {
    let keys = SENT_KEYS.get_or_init(|| Mutex::new(HashSet::new()));
    {
        let mut sent = keys
            .lock()
            .map_err(|_| "notification state lock poisoned".to_string())?;
        if !sent.insert(key.clone()) {
            return Ok(());
        }
    }
    if let Err(error) = send(app, title, body) {
        if let Ok(mut sent) = keys.lock() {
            sent.remove(&key);
        }
        return Err(error);
    }
    Ok(())
}
