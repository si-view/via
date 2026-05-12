use anyhow::{anyhow, Result};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tokio::net::UnixStream;
use tokio_util::codec::{FramedRead, FramedWrite};
use uuid::Uuid;

use crate::cli::SendArgs;
use crate::codec;
use crate::process::Registry;
use crate::proto::{EvalRequest, EvalResponse};

/// Connect to a running `via serve`, send one SKILL expression, and:
///   - (default) block until the result arrives, then print JSON to stdout.
///   - (--async)  fire-and-forget; exit immediately after sending.
///
/// When `--name` is given the socket path is resolved from the managed
/// instance registry; `--sock` is ignored.
///
/// Output format (stdout, success):
///   {"id":"…","ok":true,"type":"<skill-type>","value":…}
///
/// Exit code 0  → ok:true
/// Exit code 1  → ok:false or transport error (message to stderr)
pub async fn run(args: SendArgs) -> Result<()> {
    let sock = resolve_target(&args)?;
    let expression = build_expression(&args)?;

    if args.dry_run {
        println!("[dry-run] target  : {sock}");
        println!("[dry-run] expr    : {expression}");
        println!(
            "[dry-run] mode    : {}",
            if args.no_wait {
                "fire-and-forget"
            } else {
                "sync"
            }
        );
        return Ok(());
    }

    let stream = UnixStream::connect(&sock)
        .await
        .map_err(|e| anyhow!("connect {}: {e}", sock))?;

    let (rd, wr) = stream.into_split();
    let mut reader = FramedRead::new(rd, codec::new());
    let mut writer = FramedWrite::new(wr, codec::new());

    let id = Uuid::new_v4().to_string();
    let req = EvalRequest {
        id: id.clone(),
        expression,
        no_reply: args.no_wait,
    };

    writer
        .send(Bytes::from(serde_json::to_vec(&req)?))
        .await
        .map_err(|e| anyhow!("send request: {e}"))?;

    if args.no_wait {
        return Ok(());
    }

    match reader.next().await {
        Some(Ok(frame)) => {
            let resp: EvalResponse =
                serde_json::from_slice(&frame).map_err(|e| anyhow!("decode response: {e}"))?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
            if resp.ok {
                Ok(())
            } else {
                let reason = resp.result.reason.unwrap_or_else(|| "unknown error".into());
                Err(anyhow!("{reason}"))
            }
        }
        Some(Err(e)) => Err(anyhow!("frame error: {e}")),
        None => Err(anyhow!("connection closed before response")),
    }
}

/// Resolve sock_path from either `--name` (registry lookup) or explicit
/// `--sock`.
fn resolve_target(args: &SendArgs) -> Result<String> {
    if let Some(name) = &args.name {
        let registry = Registry::load()?;
        let inst = registry.instances.get(name).ok_or_else(|| {
            anyhow!("no managed instance named '{name}'; run `via list` to check")
        })?;
        Ok(inst.sock.to_string_lossy().into_owned())
    } else {
        Ok(args.sock.clone())
    }
}

/// Build the SKILL expression string from --eval or --load.
fn build_expression(args: &SendArgs) -> Result<String> {
    if let Some(expr) = &args.eval {
        return Ok(expr.clone());
    }
    if let Some(path) = &args.load {
        let p = path.to_string_lossy();
        // Escape backslashes and double-quotes for a SKILL string literal.
        let escaped = p.replace('\\', "\\\\").replace('"', "\\\"");
        return Ok(format!("load(\"{escaped}\")"));
    }
    // clap ArgGroup guarantees at least one is set; this is unreachable.
    Err(anyhow!("one of --eval or --load is required"))
}
