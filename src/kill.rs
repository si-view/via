use anyhow::{bail, Result};
use bytes::Bytes;
use futures_util::SinkExt;
use tokio::net::UnixStream;
use tokio_util::codec::FramedWrite;
use uuid::Uuid;

use crate::cli::KillArgs;
use crate::codec;
use crate::process::{kill_process, process_alive, Registry};
use crate::proto::EvalRequest;

/// Graceful shutdown timeout: how long to wait for Virtuoso to exit after
/// receiving the SKILL `exit()` call before giving up.
const GRACEFUL_TIMEOUT_SECS: u64 = 5;
/// Poll interval while waiting for the process to die.
const POLL_MS: u64 = 300;

pub async fn run(args: KillArgs) -> Result<()> {
    let mut registry = Registry::load()?;

    let inst = registry
        .instances
        .get(&args.name)
        .ok_or_else(|| anyhow::anyhow!("no instance named '{}'", args.name))?
        .clone();

    if !process_alive(inst.virtuoso_pid) {
        eprintln!(
            "warning: process {} is already dead; removing registry entry",
            inst.virtuoso_pid
        );
        registry.instances.remove(&args.name);
        registry.save()?;
        cleanup_sock(&inst.sock);
        return Ok(());
    }

    if args.force {
        // Bypass the bridge entirely — SIGKILL, no questions asked.
        let ret =
            unsafe { libc::kill(inst.virtuoso_pid as libc::pid_t, libc::SIGKILL) };
        if ret != 0 {
            bail!(
                "SIGKILL {} failed: {}",
                inst.virtuoso_pid,
                std::io::Error::last_os_error()
            );
        }
        println!("force-killed '{}' (pid {}, SIGKILL)", inst.name, inst.virtuoso_pid);
    } else {
        // ── Graceful path ─────────────────────────────────────────────────────
        // 1. Send SKILL `exit()` through the IPC bridge (fire-and-forget).
        //    Virtuoso will begin its own cleanup sequence and exit cleanly.
        //    The bridge socket disappears as part of that, so we do NOT wait
        //    for a response — just fire and move on.
        let sock = inst.sock.to_string_lossy().into_owned();
        match graceful_exit(&sock, &inst.secret).await {
            Ok(()) => {
                println!(
                    "sent exit() to '{}' (pid {}), waiting for shutdown…",
                    inst.name, inst.virtuoso_pid
                );
            }
            Err(e) => {
                // Bridge may already be gone (e.g. via serve crashed).
                // Fall through to SIGTERM as the next best option.
                eprintln!(
                    "warning: could not reach bridge for '{}': {e}; falling back to SIGTERM",
                    inst.name
                );
                kill_process(inst.virtuoso_pid)?;
            }
        }

        // 2. Poll until the process is gone or the timeout expires.
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_secs(GRACEFUL_TIMEOUT_SECS);
        loop {
            if !process_alive(inst.virtuoso_pid) {
                break;
            }
            if std::time::Instant::now() >= deadline {
                eprintln!(
                    "warning: '{}' (pid {}) did not exit within {}s; \
                     use `via kill --force {}` to send SIGKILL",
                    inst.name,
                    inst.virtuoso_pid,
                    GRACEFUL_TIMEOUT_SECS,
                    inst.name,
                );
                // Leave registry entry intact so the user can --force later.
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(POLL_MS)).await;
        }

        println!("'{}' exited cleanly", inst.name);
    }

    registry.instances.remove(&args.name);
    registry.save()?;

    // Clean up the socket file left behind by via serve.
    cleanup_sock(&inst.sock);

    Ok(())
}

/// Remove the Unix socket file if it still exists.
fn cleanup_sock(sock: &std::path::Path) {
    if sock.exists() {
        if let Err(e) = std::fs::remove_file(sock) {
            eprintln!("warning: could not remove socket {}: {e}", sock.display());
        }
    }
}
async fn graceful_exit(sock: &str, secret: &str) -> Result<()> {
    let stream = UnixStream::connect(sock).await?;
    let (_rd, wr) = stream.into_split();
    let mut writer = FramedWrite::new(wr, codec::new());

    let req = EvalRequest {
        id: Uuid::new_v4().to_string(),
        secret: secret.to_owned(),
        expression: "exit()".to_owned(),
        no_reply: true,
    };
    writer
        .send(Bytes::from(serde_json::to_vec(&req)?))
        .await?;
    Ok(())
}
