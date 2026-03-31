/// via serve — bridge server
///
/// Concurrency model:
///
///   via send (N clients)
///       │  Unix socket  [4B-LE-len | JSON]
///       ▼
///   handle_client task(s)
///       │  mpsc  PendingReq { id, expression, reply_tx? }
///       ▼
///   router_task  ── stdout ──► Virtuoso (one expression at a time, serial)
///       ▲
///       │  mpsc  Result<Value, String>
///   cb_reader_task
///       ▲
///   callback Unix socket  ◄── via forward ◄── Virtuoso ipcWriteProcess
///
/// The router is the only writer to stdout.  All other tasks must not write
/// to stdout or stderr (Virtuoso would try to evaluate anything on stdout).
use anyhow::{Context, Result};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};
use tracing::{debug, error, info, warn};

use crate::cli::ServeArgs;
use crate::codec;
use crate::proto::{EvalRequest, EvalResponse, Outcome};

// ── types ─────────────────────────────────────────────────────────────────────

type ReplyTx = oneshot::Sender<Result<Value, String>>;
type ClientWriter = FramedWrite<OwnedWriteHalf, LengthDelimitedCodec>;

struct PendingReq {
    id: String,
    expression: String,
    reply_tx: Option<ReplyTx>,
}

// ── entry point ───────────────────────────────────────────────────────────────

pub async fn run(args: ServeArgs) -> Result<()> {
    // File logging must be set up before any tracing macros.
    // stdout/stderr are captured by Virtuoso — do not write to them except
    // through the router (SKILL expressions only).
    let _guard = crate::log::init_file(&args.log_file)?;

    let secret = Arc::new(args.secret.clone());

    // ── channels ──────────────────────────────────────────────────────────────
    let (req_tx, req_rx) = mpsc::channel::<PendingReq>(256);
    let (cb_tx, cb_rx) = mpsc::channel::<Result<Value, String>>(256);

    // ── callback socket (serve binds; forward connects) ───────────────────────
    remove_if_exists(&args.cb_sock)?;
    let cb_listener = UnixListener::bind(&args.cb_sock)
        .with_context(|| format!("bind cb_sock {}", args.cb_sock))?;
    info!("[si-view] callback socket: {}", args.cb_sock);

    // ── client socket (for `via send`) ────────────────────────────────────────
    remove_if_exists(&args.sock)?;
    let listener = UnixListener::bind(&args.sock)
        .with_context(|| format!("bind sock {}", args.sock))?;
    info!("[si-view] client socket: {}", args.sock);

    // ── background tasks ──────────────────────────────────────────────────────
    tokio::spawn(router_task(req_rx, cb_rx));
    tokio::spawn(cb_accept_task(cb_listener, args.cb_token.clone(), cb_tx));

    // ── accept loop ───────────────────────────────────────────────────────────
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                tokio::spawn(handle_client(stream, req_tx.clone(), secret.clone()));
            }
            Err(e) => error!("[si-view] accept: {e}"),
        }
    }
}

// ── router ────────────────────────────────────────────────────────────────────

/// Serial dispatcher: one SKILL expression in-flight at a time.
///
/// Virtuoso evaluates expressions sequentially; the router enforces this by
/// only writing the next expression to stdout once the callback response for
/// the current one has arrived.
async fn router_task(
    mut req_rx: mpsc::Receiver<PendingReq>,
    mut cb_rx: mpsc::Receiver<Result<Value, String>>,
) {
    let mut queue: VecDeque<PendingReq> = VecDeque::new();
    let mut in_flight: Option<(String, Option<ReplyTx>)> = None;

    // The only legitimate writer to stdout in the entire process.
    let mut out = tokio::io::BufWriter::new(tokio::io::stdout());

    loop {
        // While idle, drain the queue.
        if in_flight.is_none() {
            if let Some(req) = queue.pop_front() {
                let line = format!("{}\n", req.expression);
                if out.write_all(line.as_bytes()).await.is_err()
                    || out.flush().await.is_err()
                {
                    error!("[si-view] stdout write failed");
                    break;
                }
                debug!("[si-view] → Virtuoso: {}", req.expression);
                in_flight = Some((req.id, req.reply_tx));
            }
        }

        tokio::select! {
            msg = req_rx.recv() => match msg {
                Some(r) => queue.push_back(r),
                None => { info!("[si-view] req channel closed, router exiting"); break; }
            },
            resp = cb_rx.recv() => match resp {
                Some(r) => {
                    if let Some((_id, tx)) = in_flight.take() {
                        if let Some(tx) = tx {
                            let _ = tx.send(r);
                        }
                    } else {
                        warn!("[si-view] callback response with no in-flight request — discarded");
                    }
                }
                None => { info!("[si-view] cb channel closed, router exiting"); break; }
            },
        }
    }
}

// ── callback socket ───────────────────────────────────────────────────────────

async fn cb_accept_task(
    listener: UnixListener,
    token: String,
    cb_tx: mpsc::Sender<Result<Value, String>>,
) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                debug!("[si-view] forwarder connected");
                tokio::spawn(cb_reader_task(stream, token.clone(), cb_tx.clone()));
            }
            Err(e) => {
                error!("[si-view] cb accept: {e}");
                break;
            }
        }
    }
}

/// Read `<TOKEN> S:JSON\n` / `<TOKEN> F:error\n` / `<TOKEN> E:{…}\n` lines
/// from the forwarder and forward results to the router.
async fn cb_reader_task(
    stream: UnixStream,
    token: String,
    cb_tx: mpsc::Sender<Result<Value, String>>,
) {
    let prefix = format!("{token} ");
    let mut lines = BufReader::new(stream).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let rest = match line.strip_prefix(&prefix) {
            Some(r) => r,
            None => {
                warn!("[si-view] callback: rejected line (bad token)");
                continue;
            }
        };

        if let Some(json) = rest.strip_prefix("S:") {
            let val = serde_json::from_str::<Value>(json)
                .map_err(|e| format!("JSON parse: {e}"));
            let _ = cb_tx.send(val).await;
        } else if let Some(err) = rest.strip_prefix("F:") {
            let _ = cb_tx.send(Err(err.to_string())).await;
        } else if let Some(payload) = rest.strip_prefix("E:") {
            // Events have no subscriber in the current design; log them.
            debug!("[si-view] event: {payload}");
        } else {
            warn!("[si-view] callback: unknown prefix in '{rest}'");
        }
    }

    debug!("[si-view] forwarder disconnected");
}

// ── client handler ────────────────────────────────────────────────────────────

async fn handle_client(
    stream: UnixStream,
    req_tx: mpsc::Sender<PendingReq>,
    secret: Arc<String>,
) {
    let (rd, wr) = stream.into_split();
    let mut reader = FramedRead::new(rd, codec::new());
    let mut writer = FramedWrite::new(wr, codec::new());

    while let Some(frame) = reader.next().await {
        let frame = match frame {
            Ok(f) => f,
            Err(e) => {
                error!("[si-view] frame decode: {e}");
                break;
            }
        };

        let req: EvalRequest = match serde_json::from_slice(&frame) {
            Ok(r) => r,
            Err(e) => {
                error!("[si-view] JSON decode: {e}");
                break;
            }
        };

        // ── authentication ────────────────────────────────────────────────────
        if !secret.is_empty() && req.secret != *secret {
            warn!("[si-view] rejected id={} (bad secret)", req.id);
            reply(
                &mut writer,
                EvalResponse {
                    id: req.id,
                    outcome: Outcome::Failure {
                        error: "authentication failed".into(),
                    },
                },
            )
            .await;
            break;
        }

        // ── dispatch to router ────────────────────────────────────────────────
        let (reply_tx, reply_rx) = if req.no_reply {
            (None, None)
        } else {
            let (tx, rx) = oneshot::channel();
            (Some(tx), Some(rx))
        };

        let id = req.id.clone();
        if req_tx
            .send(PendingReq {
                id: req.id,
                expression: req.expression,
                reply_tx,
            })
            .await
            .is_err()
        {
            error!("[si-view] router channel closed");
            break;
        }

        // ── wait for result (sync mode) ───────────────────────────────────────
        if let Some(rx) = reply_rx {
            let outcome = match rx.await {
                Ok(Ok(val)) => Outcome::Success { result: val },
                Ok(Err(err)) => Outcome::Failure { error: err },
                Err(_) => {
                    error!("[si-view] reply channel dropped");
                    break;
                }
            };
            reply(&mut writer, EvalResponse { id, outcome }).await;
        }
    }
}

async fn reply(writer: &mut ClientWriter, resp: EvalResponse) {
    match serde_json::to_vec(&resp) {
        Ok(bytes) => {
            if let Err(e) = writer.send(Bytes::from(bytes)).await {
                error!("[si-view] send response: {e}");
            }
        }
        Err(e) => error!("[si-view] serialize response: {e}"),
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn remove_if_exists(path: &str) -> Result<()> {
    let p = std::path::Path::new(path);
    if p.exists() {
        std::fs::remove_file(p).with_context(|| format!("remove {path}"))?;
    }
    Ok(())
}
