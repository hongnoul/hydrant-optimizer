//! Durable user selections, separate from the replaceable catalog cache.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::storage;

const VERSION: u32 = 1;
const MAX_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedSelection {
    selected: BTreeSet<String>,
    saved_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sessions {
    version: u32,
    terms: BTreeMap<String, SavedSelection>,
}

/// Holds an advisory lock for the entire TUI lifetime. The OS releases it even
/// on SIGTERM/SIGKILL, unlike create-new PID files. Never unlink the lock file:
/// replacing its inode would allow two writers to hold different locks.
#[derive(Debug)]
pub struct SelectionSession {
    _lock: File,
    path: PathBuf,
    sessions: Sessions,
    disk_bytes: Option<Vec<u8>>,
}

impl SelectionSession {
    pub fn open(dir: &Path) -> Result<Self> {
        fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        let lock_path = dir.join("sessions.lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| format!("open {}", lock_path.display()))?;
        fs2::FileExt::try_lock_exclusive(&lock).with_context(|| {
            "another TUI may be using this data directory; use --no-restore for an ephemeral session"
        })?;
        let path = dir.join("sessions.json");
        let disk_bytes = read_bytes(&path)?;
        let sessions = match &disk_bytes {
            Some(bytes) => {
                // Inspect the version before decoding the version-specific schema.
                let value: serde_json::Value = serde_json::from_slice(bytes)
                    .with_context(|| format!("parse {} (file preserved)", path.display()))?;
                ensure!(
                    value.get("version").and_then(|v| v.as_u64()) == Some(VERSION as u64),
                    "unsupported sessions.json version (file preserved)"
                );
                let sessions: Sessions = serde_json::from_value(value)
                    .context("invalid sessions.json schema (file preserved)")?;
                for (term, saved) in &sessions.terms {
                    validate_id(term)?;
                    for id in &saved.selected {
                        validate_id(id)?;
                    }
                    DateTime::parse_from_rfc3339(&saved.saved_at)
                        .context("invalid session save timestamp (file preserved)")?;
                }
                sessions
            }
            None => Sessions {
                version: VERSION,
                terms: BTreeMap::new(),
            },
        };
        Ok(Self {
            _lock: lock,
            path,
            sessions,
            disk_bytes,
        })
    }

    pub fn selected(&self, term: &str) -> Option<&BTreeSet<String>> {
        self.sessions.terms.get(term).map(|s| &s.selected)
    }

    /// Do not update the in-memory baseline until the replacement succeeds.
    /// Repeated identical state is a no-op, including on normal exit.
    pub fn save(&mut self, term: &str, selected: &BTreeSet<String>) -> Result<()> {
        validate_id(term)?;
        for id in selected {
            validate_id(id)?;
        }
        ensure!(
            read_bytes(&self.path)? == self.disk_bytes,
            "sessions.json changed outside this TUI; restart to reload it (file preserved)"
        );
        if self.selected(term) == Some(selected) {
            return Ok(());
        }
        let mut next = self.sessions.clone();
        next.terms.insert(
            term.to_owned(),
            SavedSelection {
                selected: selected.clone(),
                saved_at: Utc::now().to_rfc3339(),
            },
        );
        let bytes = serde_json::to_vec_pretty(&next)?;
        ensure!(
            bytes.len() as u64 <= MAX_BYTES,
            "session file exceeds size limit"
        );
        storage::atomic_write(&self.path, &bytes).context("write sessions.json")?;
        self.sessions = next;
        self.disk_bytes = Some(bytes);
        Ok(())
    }
}

fn validate_id(id: &str) -> Result<()> {
    ensure!(
        !id.trim().is_empty() && !id.chars().any(char::is_control),
        "invalid term or subject ID in session state (file preserved)"
    );
    Ok(())
}

fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "session file exceeds size limit (file preserved)"
    );
    Ok(Some(bytes))
}
