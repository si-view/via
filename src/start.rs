use anyhow::{bail, Context, Result};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use uuid::Uuid;

use crate::cli::StartArgs;
use crate::process::{now_string, process_alive, Instance, Registry};

pub fn run(args: StartArgs) -> Result<()> {
    // ── 1. Require DISPLAY ────────────────────────────────────────────────────
    if std::env::var("DISPLAY").is_err() {
        bail!("DISPLAY environment variable is not set; Virtuoso requires a display");
    }

    // ── 2. Validate name ──────────────────────────────────────────────────────
    validate_name(&args.name)?;

    let mut registry = Registry::load()?;
    if let Some(existing) = registry.instances.get(&args.name) {
        if process_alive(existing.virtuoso_pid) {
            bail!(
                "instance '{}' is already running (pid {})",
                args.name,
                existing.virtuoso_pid
            );
        }
        // Stale entry from a previously dead process — remove it and continue.
        registry.instances.remove(&args.name);
    }

    // ── 3. Resolve paths ──────────────────────────────────────────────────────
    let user = std::env::var("USER").unwrap_or_else(|_| "user".to_string());

    let workspace = match args.workspace {
        Some(p) => p.canonicalize().context("resolve --workspace path")?,
        None => std::env::current_dir().context("get current directory")?,
    };

    let via_dir = Registry::dir();
    let log_dir = via_dir.join("logs");
    let tmp_dir = via_dir.join("tmp");
    std::fs::create_dir_all(&log_dir).context("create ~/.via/logs")?;
    std::fs::create_dir_all(&tmp_dir).context("create ~/.via/tmp")?;

    let sock = PathBuf::from(format!("/tmp/via-{user}-{}.sock", args.name));
    let virtuoso_log = log_dir.join(format!("{}-virtuoso.log", args.name));
    let via_log = log_dir.join(format!("{}-via.log", args.name));
    let il_file = tmp_dir.join(format!("{}-restore.il", args.name));

    // ── 4. Locate via binary and via.il ───────────────────────────────────────
    let binary_path = std::env::current_exe().context("locate via binary")?;

    let via_il_path = match args.via_il {
        Some(p) => p,
        None => {
            let candidate = binary_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("via.il");
            if !candidate.exists() {
                bail!(
                    "via.il not found at {}; use --via-il to specify its path",
                    candidate.display()
                );
            }
            candidate
        }
    };

    // ── 5. Generate shared secret ─────────────────────────────────────────────
    // Two UUIDs (no hyphens) = 64-char hex string.
    let secret = format!(
        "{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    );

    // ── 6. Write IL restore file ──────────────────────────────────────────────
    let il_content = build_il(
        &via_il_path,
        &binary_path,
        &sock,
        &secret,
        &via_log,
    );
    std::fs::write(&il_file, &il_content)
        .with_context(|| format!("write IL restore file {}", il_file.display()))?;

    // ── 7. Launch Virtuoso detached ───────────────────────────────────────────
    let virtuoso_stdout = std::fs::File::create(&virtuoso_log)
        .with_context(|| format!("create virtuoso log {}", virtuoso_log.display()))?;
    let virtuoso_stderr = virtuoso_stdout.try_clone()?;

    let mut cmd = Command::new(&args.virtuoso);
    cmd.args(["-nograph", "-restore"])
        .arg(&il_file)
        .current_dir(&workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::from(virtuoso_stdout))
        .stderr(Stdio::from(virtuoso_stderr))
        // Put the child in its own process group so it outlives our process.
        .process_group(0);

    let child = cmd
        .spawn()
        .with_context(|| format!("spawn virtuoso binary '{}'", args.virtuoso))?;
    let pid = child.id();
    // Intentionally drop the Child handle without waiting — the process runs
    // in the background.  Dropping without wait() on Unix does not kill the
    // child; it merely leaks the wait-slot until init reaps it.
    drop(child);

    // ── 8. Persist registry entry ─────────────────────────────────────────────
    registry.instances.insert(
        args.name.clone(),
        Instance {
            name: args.name.clone(),
            virtuoso_pid: pid,
            sock: sock.clone(),
            secret,
            workspace,
            virtuoso_log,
            via_log,
            started_at: now_string(),
        },
    );
    registry.save()?;

    println!(
        "started '{}'  pid={}  sock={}",
        args.name,
        pid,
        sock.display()
    );
    Ok(())
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Only allow names that are safe to embed in file paths and socket paths.
fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("--name must not be empty");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("--name may only contain ASCII letters, digits, '-', and '_'");
    }
    Ok(())
}

/// Escape a string for embedding inside a SKILL double-quoted string literal.
fn escape_il(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

/// Generate the SKILL restore file that loads via.il and calls si_view_start.
fn build_il(
    via_il: &PathBuf,
    binary: &PathBuf,
    sock: &PathBuf,
    secret: &str,
    via_log: &PathBuf,
) -> String {
    format!(
        r#"load("{via_il}")
si_view_start("{binary}"
  ?sock    "{sock}"
  ?secret  "{secret}"
  ?log_file "{via_log}"
)
"#,
        via_il = escape_il(&via_il.to_string_lossy()),
        binary = escape_il(&binary.to_string_lossy()),
        sock = escape_il(&sock.to_string_lossy()),
        secret = escape_il(secret),
        via_log = escape_il(&via_log.to_string_lossy()),
    )
}
