use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// A single tracked Virtuoso instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instance {
    pub name: String,
    pub virtuoso_pid: u32,
    /// Unix socket path that `via serve` listens on.
    pub sock: PathBuf,
    pub workspace: PathBuf,
    /// File where Virtuoso's stdout/stderr is redirected.
    pub virtuoso_log: PathBuf,
    /// File where `via serve` writes its structured log.
    pub via_log: PathBuf,
    pub started_at: String,
}

/// Persisted registry of all managed instances.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Registry {
    pub instances: HashMap<String, Instance>,
}

impl Registry {
    /// Base directory: `~/.via/`
    pub fn dir() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(home).join(".via")
    }

    pub fn path() -> PathBuf {
        Self::dir().join("registry.json")
    }

    pub fn load() -> Result<Self> {
        let path = Self::path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("read registry {}", path.display()))?;
        serde_json::from_str(&content).context("parse registry JSON")
    }

    /// Atomic write: write to a `.tmp` file then rename.
    pub fn save(&self) -> Result<()> {
        let dir = Self::dir();
        std::fs::create_dir_all(&dir).context("create ~/.via")?;
        let path = Self::path();
        let tmp = path.with_extension("json.tmp");
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&tmp, content)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

/// Check whether a process is still alive by sending signal 0.
pub fn process_alive(pid: u32) -> bool {
    // SAFETY: kill(pid, 0) is a read-only existence check, no side effects.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

/// Read the entire contents of `path`, delete the file, and return the value.
/// Trailing newlines are stripped so callers get a clean token string.
/// Used for `--cb-token-file` to keep the callback token out of the process
/// argument list.
pub fn read_and_delete(path: &std::path::Path) -> Result<String> {
    let value =
        std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    std::fs::remove_file(path).with_context(|| format!("delete {}", path.display()))?;
    Ok(value.trim_end_matches('\n').to_owned())
}

/// Send SIGTERM to a process.
pub fn kill_process(pid: u32) -> Result<()> {
    let ret = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    if ret == 0 {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "kill({}): {}",
            pid,
            std::io::Error::last_os_error()
        ))
    }
}

/// Readable timestamp for registry entries.
pub fn now_string() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    // Format as YYYY-MM-DD HH:MM:SS UTC without pulling in chrono.
    let s = secs;
    let (y, mo, d, h, mi, sec) = epoch_to_parts(s);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{sec:02} UTC")
}

fn epoch_to_parts(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let sec = secs % 60;
    let mins = secs / 60;
    let min = mins % 60;
    let hours = mins / 60;
    let hour = hours % 24;
    let days = hours / 24;

    // Gregorian calendar: number of days since 1970-01-01
    let mut y = 1970u64;
    let mut rem = days;
    loop {
        let dy = if is_leap(y) { 366 } else { 365 };
        if rem < dy {
            break;
        }
        rem -= dy;
        y += 1;
    }
    let months = [
        31u64,
        if is_leap(y) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut mo = 1u64;
    for &dm in &months {
        if rem < dm {
            break;
        }
        rem -= dm;
        mo += 1;
    }
    (y, mo, rem + 1, hour, min, sec)
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}
