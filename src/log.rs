use anyhow::{anyhow, Result};
use std::path::Path;
use tracing_appender::non_blocking::WorkerGuard;

/// Initialise file-based tracing.  Returns a guard that must be kept alive for
/// the duration of the process — dropping it flushes the background writer.
///
/// stdout and stderr are intentionally left untouched: Virtuoso captures both
/// from child processes started with ipcBeginProcess and tries to evaluate
/// any output as a SKILL expression.
pub fn init_file(path: &Path) -> Result<WorkerGuard> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| anyhow!("invalid log path: {}", path.display()))?;

    let appender = tracing_appender::rolling::never(parent, name);
    let (non_blocking, guard) = tracing_appender::non_blocking(appender);

    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(false)
        .init();

    Ok(guard)
}
