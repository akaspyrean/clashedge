//! Persisted rolling node availability and latency scores.
//!
//! A session probes each node against three independent public endpoints. At
//! least two successful points count as an available session; the score uses the
//! last 50 sessions per node and is stored under the app data directory.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::util::error::{Error, Result};

const HISTORY_FILE: &str = "node-health.json";
const HISTORY_VERSION: u32 = 2;
const MAX_SESSIONS_PER_NODE: usize = 50;
const MAX_TRACKED_NODES: usize = 500;
const MAX_NODE_NAME_BYTES: usize = 256;
const REQUIRED_SUCCESSFUL_POINTS: usize = 2;
pub const PROBE_ENDPOINTS: &[&str] = &[
    "http://www.gstatic.com/generate_204",
    "https://cp.cloudflare.com/generate_204",
    "https://www.msftconnecttest.com/connecttest.txt",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeProbePoint {
    pub url: String,
    pub delay_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProbeSession {
    tested_at_ms: u64,
    points: Vec<NodeProbePoint>,
}

impl ProbeSession {
    fn successful_points(&self) -> usize {
        self.points
            .iter()
            .filter(|point| point.delay_ms.is_some_and(|delay| delay > 0))
            .count()
    }

    fn available(&self) -> bool {
        self.successful_points() >= REQUIRED_SUCCESSFUL_POINTS
    }

    fn median_latency_ms(&self) -> Option<u64> {
        median_latency(&self.points)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct HistoryFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    nodes: HashMap<String, VecDeque<ProbeSession>>,
}

/// Summary returned to the UI and used for node ranking.
#[derive(Debug, Clone, Serialize)]
pub struct NodeHealthSummary {
    pub node: String,
    /// Combined 0..100 score: 70% rolling availability, 30% median latency.
    pub score: f64,
    /// Successful sessions divided by retained sessions (0..1).
    pub availability: f64,
    /// Median of successful endpoint latency samples across retained sessions.
    pub median_latency_ms: Option<u64>,
    pub sessions: usize,
    pub last_available: bool,
    pub last_latency_ms: Option<u64>,
    pub last_tested_at_ms: u64,
}

/// Thread-safe owner for the local rolling history.
pub struct NodeHealthHistory {
    path: PathBuf,
    file: Mutex<HistoryFile>,
}

impl NodeHealthHistory {
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join(HISTORY_FILE);
        let mut file = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<HistoryFile>(&bytes) {
                Ok(file) => file,
                Err(error) => {
                    warn!("Ignoring invalid node health history: {}", error);
                    HistoryFile::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => HistoryFile::default(),
            Err(error) => {
                warn!("Unable to read node health history: {}", error);
                HistoryFile::default()
            }
        };
        if file.version != HISTORY_VERSION {
            // Version 2 scopes records by profile; older node-only keys cannot be
            // attributed safely, so discard them instead of mixing subscriptions.
            file = HistoryFile::default();
        }
        for sessions in file.nodes.values_mut() {
            while sessions.len() > MAX_SESSIONS_PER_NODE {
                sessions.pop_front();
            }
        }
        if file.nodes.len() > MAX_TRACKED_NODES {
            let mut names: Vec<_> = file
                .nodes
                .iter()
                .map(|(name, sessions)| {
                    (
                        name.clone(),
                        sessions.back().map(|session| session.tested_at_ms).unwrap_or(0),
                    )
                })
                .collect();
            names.sort_by_key(|(_, tested_at)| *tested_at);
            for (name, _) in names.into_iter().take(file.nodes.len() - MAX_TRACKED_NODES) {
                file.nodes.remove(&name);
            }
        }
        file.version = HISTORY_VERSION;
        Self {
            path,
            file: Mutex::new(file),
        }
    }

    pub fn record(
        &self,
        profile: &str,
        node: &str,
        points: Vec<NodeProbePoint>,
    ) -> Result<NodeHealthSummary> {
        if node.trim().is_empty() || node.len() > MAX_NODE_NAME_BYTES {
            return Err(Error::Other("invalid node name for health history".to_string()));
        }
        let history_key = history_key(profile, node);
        let mut file = self.file.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let sessions = file.nodes.entry(history_key.clone()).or_default();
        sessions.push_back(ProbeSession {
            tested_at_ms: now_ms(),
            points,
        });
        while sessions.len() > MAX_SESSIONS_PER_NODE {
            sessions.pop_front();
        }

        let summary = summarize(node, sessions);
        while file.nodes.len() > MAX_TRACKED_NODES {
            let oldest = file
                .nodes
                .iter()
                .filter(|(name, _)| name.as_str() != history_key.as_str())
                .min_by_key(|(_, history)| {
                    history.back().map(|session| session.tested_at_ms).unwrap_or(0)
                })
                .map(|(name, _)| name.clone());
            if let Some(oldest) = oldest {
                file.nodes.remove(&oldest);
            } else {
                break;
            }
        }

        let bytes = serde_json::to_vec(&*file)
            .map_err(|error| Error::Other(format!("serialize node health history: {}", error)))?;
        crate::util::atomic::atomic_write(&self.path, &bytes)?;
        Ok(summary)
    }

    pub fn summaries(&self, profile: &str, names: Option<&[String]>) -> Vec<NodeHealthSummary> {
        let file = self.file.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let prefix = history_prefix(profile);
        let mut summaries: Vec<_> = file
            .nodes
            .iter()
            .filter_map(|(key, sessions)| {
                let node = key.strip_prefix(&prefix)?;
                if names.is_some_and(|names| !names.iter().any(|name| name == node)) {
                    return None;
                }
                Some(summarize(node, sessions))
            })
            .collect();
        summaries.sort_by(|a, b| b.score.total_cmp(&a.score));
        summaries
    }
}

fn history_prefix(profile: &str) -> String {
    format!("{}:{}", profile.len(), profile)
}

fn history_key(profile: &str, node: &str) -> String {
    format!("{}{}", history_prefix(profile), node)
}

fn summarize(node: &str, sessions: &VecDeque<ProbeSession>) -> NodeHealthSummary {
    let successful_sessions = sessions.iter().filter(|session| session.available()).count();
    let availability = if sessions.is_empty() {
        0.0
    } else {
        successful_sessions as f64 / sessions.len() as f64
    };
    let all_points: Vec<NodeProbePoint> = sessions
        .iter()
        .flat_map(|session| session.points.iter().cloned())
        .collect();
    let median_latency_ms = median_latency(&all_points);
    let latency_score = median_latency_ms
        .map(|latency| 1000.0 / (1000.0 + latency as f64) * 100.0)
        .unwrap_or(0.0);
    let last = sessions.back();
    NodeHealthSummary {
        node: node.to_string(),
        score: availability * 70.0 + latency_score * 0.30,
        availability,
        median_latency_ms,
        sessions: sessions.len(),
        last_available: last.is_some_and(ProbeSession::available),
        last_latency_ms: last.and_then(ProbeSession::median_latency_ms),
        last_tested_at_ms: last.map(|session| session.tested_at_ms).unwrap_or(0),
    }
}

fn median_latency(points: &[NodeProbePoint]) -> Option<u64> {
    let mut values: Vec<u64> = points
        .iter()
        .filter_map(|point| point.delay_ms)
        .filter(|delay| *delay > 0)
        .collect();
    values.sort_unstable();
    match values.len() {
        0 => None,
        len if len % 2 == 1 => Some(values[len / 2]),
        len => {
            let lower = values[len / 2 - 1];
            let upper = values[len / 2];
            Some(lower + (upper - lower) / 2)
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}
