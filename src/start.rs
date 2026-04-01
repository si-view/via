use anyhow::{bail, Context, Result};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use uuid::Uuid;

use crate::cli::StartArgs;
use crate::process::{now_string, process_alive, Instance, Registry};

/// Compiled SKILL context — embedded at compile time, never touches disk as
/// plain-text source.
const VIA_CXT: &[u8] = include_bytes!("../via.cxt");

pub fn run(args: StartArgs) -> Result<()> {
    // ── 1. Require DISPLAY ────────────────────────────────────────────────────
    if std::env::var("DISPLAY").is_err() {
        bail!("DISPLAY environment variable is not set; Virtuoso requires a display");
    }

    // ── 2. Validate name and check registry ───────────────────────────────────
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
        // Stale entry from a previously dead process — reclaim the name.
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
    let cxt_dir = tmp_dir.join("64bit");
    std::fs::create_dir_all(&log_dir).context("create ~/.via/logs")?;
    std::fs::create_dir_all(&cxt_dir).context("create ~/.via/tmp/64bit")?;

    let sock = PathBuf::from(format!("/tmp/via-{user}-{}.sock", args.name));
    let virtuoso_log = log_dir.join(format!("{}-virtuoso.log", args.name));
    let via_log = log_dir.join(format!("{}-via.log", args.name));

    // ── 4. Locate the via binary ──────────────────────────────────────────────
    let binary_path = std::env::current_exe().context("locate via binary")?;

    // ── 5. Generate shared secret and per-start random file names ─────────────
    // Two UUID v4s without hyphens → 64-char hex string.
    let secret = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    // Random hex stems — different on every `via start` invocation.
    let cxt_stem = Uuid::new_v4().simple().to_string();
    let il_stem = Uuid::new_v4().simple().to_string();

    let cxt_file = cxt_dir.join(&cxt_stem);          // no extension — Cadence convention
    let il_file = tmp_dir.join(format!("{il_stem}.il"));

    // ── 6. Write compiled context and bootstrap IL ────────────────────────────
    if args.dry_run {
        println!("[dry-run] instance name : {}", args.name);
        println!("[dry-run] workspace     : {}", workspace.display());
        println!("[dry-run] sock          : {}", sock.display());
        println!("[dry-run] virtuoso log  : {}", virtuoso_log.display());
        println!("[dry-run] via log       : {}", via_log.display());
        println!("[dry-run] virtuoso      : {}{}", args.virtuoso, if args.nograph { " -nograph" } else { "" });
        return Ok(());
    }

    std::fs::write(&cxt_file, VIA_CXT)
        .with_context(|| format!("write context file {}", cxt_file.display()))?;

    let il_content = build_il(&cxt_file, &il_file, &binary_path, &sock, &secret, &via_log);
    std::fs::write(&il_file, &il_content)
        .with_context(|| format!("write IL bootstrap {}", il_file.display()))?;

    // ── 7. Launch Virtuoso detached ───────────────────────────────────────────
    let virtuoso_stdout = std::fs::File::create(&virtuoso_log)
        .with_context(|| format!("create virtuoso log {}", virtuoso_log.display()))?;
    let virtuoso_stderr = virtuoso_stdout.try_clone()?;

    let mut cmd = Command::new(&args.virtuoso);
    cmd.arg("-restore")
        .arg(&il_file);
    if args.nograph {
        cmd.arg("-nograph");
    }
    cmd.current_dir(&workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::from(virtuoso_stdout))
        .stderr(Stdio::from(virtuoso_stderr))
        // New process group — child outlives the via process.
        .process_group(0);

    let child = cmd
        .spawn()
        .with_context(|| format!("spawn virtuoso binary '{}'", args.virtuoso))?;
    let pid = child.id();
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

/// Build the self-deleting IL bootstrap file.
///
/// Execution order inside Virtuoso:
///   1. `loadContext` — loads compiled SKILL definitions from the `.cxt` binary.
///   2. `si_view_start` — launches `via serve` + `via forward`.
///   3. `deleteFile` × 2 — removes the context binary and this file from disk.
fn build_il(
    cxt_file: &PathBuf,
    il_file: &PathBuf,
    binary: &PathBuf,
    sock: &PathBuf,
    secret: &str,
    via_log: &PathBuf,
) -> String {
    format!(
        r#"loadContext("{cxt}")
si_view_start("{binary}"
  ?sock     "{sock}"
  ?secret   "{secret}"
  ?log_file "{via_log}"
)
deleteFile("{cxt}")
deleteFile("{il}")
"#,
        cxt    = escape_il(&cxt_file.to_string_lossy()),
        binary = escape_il(&binary.to_string_lossy()),
        sock   = escape_il(&sock.to_string_lossy()),
        secret = escape_il(secret),
        via_log = escape_il(&via_log.to_string_lossy()),
        il     = escape_il(&il_file.to_string_lossy()),
    )
}
