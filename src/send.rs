use anyhow::{anyhow, Result};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tokio::net::UnixStream;
use tokio_util::codec::{FramedRead, FramedWrite};
use uuid::Uuid;

use crate::cli::SendArgs;
use crate::codec;
use crate::proto::{EvalRequest, EvalResponse, Outcome};

/// Connect to a running `via serve`, send one SKILL expression, and:
///   - (default) block until the result arrives, then print JSON to stdout.
///   - (--async)  fire-and-forget; exit immediately after sending.
///
/// Exit code 0  → success (result printed to stdout)
/// Exit code 1  → SKILL evaluation failure or transport error (message to stderr)
pub async fn run(args: SendArgs) -> Result<()> {
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
        expression: args.expr,
        no_reply: args.no_wait,
    };

    writer
        .send(Bytes::from(serde_json::to_vec(&req)?))
        .await
        .map_err(|e| anyhow!("send request: {e}"))?;

    if args.no_wait {
        // Fire-and-forget: SinkExt::send already flushed.
        return Ok(());
    }

    match reader.next().await {
        Some(Ok(frame)) => {
            let resp: EvalResponse = serde_json::from_slice(&frame)
                .map_err(|e| anyhow!("decode response: {e}"))?;
            match resp.outcome {
                Outcome::Success { result } => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                    Ok(())
                }
                Outcome::Failure { error } => Err(anyhow!("{error}")),
            }
        }
        Some(Err(e)) => Err(anyhow!("frame error: {e}")),
        None => Err(anyhow!("connection closed before response")),
    }
}
