use anyhow::{anyhow, Result};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tokio::net::UnixStream;
use tokio_util::codec::{FramedRead, FramedWrite};
use uuid::Uuid;

use crate::cli::SendArgs;
use crate::codec;
use crate::proto::{EvalRequest, EvalResponse};

/// Connect to a running `via serve`, send one SKILL expression, and:
///   - (default) block until the result arrives, then print JSON to stdout.
///   - (--async)  fire-and-forget; exit immediately after sending.
///
/// Output format (stdout, success):
///   {"id":"…","ok":true,"type":"<skill-type>","value":…}
///
/// Exit code 0  → ok:true
/// Exit code 1  → ok:false or transport error (message to stderr)
pub async fn run(args: SendArgs) -> Result<()> {
    let expression = build_expression(&args)?;

    let stream = UnixStream::connect(&args.sock)
        .await
        .map_err(|e| anyhow!("connect {}: {e}", args.sock))?;

    let (rd, wr) = stream.into_split();
    let mut reader = FramedRead::new(rd, codec::new());
    let mut writer = FramedWrite::new(wr, codec::new());

    let id = Uuid::new_v4().to_string();
    let req = EvalRequest {
        id: id.clone(),
        secret: args.secret,
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
            let resp: EvalResponse = serde_json::from_slice(&frame)
                .map_err(|e| anyhow!("decode response: {e}"))?;
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
