use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::time::{sleep, Duration};
use tracing::{info, warn};

use crate::cli::ForwardArgs;

/// Read Virtuoso's ipcWriteProcess output line-by-line from stdin, prepend the
/// shared cb-token, and relay each line to the callback socket.
///
/// Connects with exponential back-off so it can start fractionally before
/// `via serve` has created the socket.
pub async fn run(args: ForwardArgs) -> Result<()> {
    let _guard = crate::log::init_file(&args.log_file)?;

    let cb_token = if let Some(path) = &args.cb_token_file {
        crate::process::read_and_delete(path)?
    } else {
        args.cb_token
            .clone()
            .ok_or_else(|| anyhow::anyhow!("one of --cb-token or --cb-token-file is required"))?
    };

    let stream = connect_with_retry(&args.cb_sock).await?;
    let mut writer = tokio::io::BufWriter::new(stream);

    let mut lines = BufReader::new(tokio::io::stdin()).lines();

    while let Some(line) = lines.next_line().await? {
        // Protocol: "<TOKEN> <original-line>\n"
        let msg = format!("{} {}\n", cb_token, line);
        writer.write_all(msg.as_bytes()).await?;
        writer.flush().await?;
    }

    info!("[si-view] stdin closed, forwarder exiting");
    Ok(())
}

async fn connect_with_retry(path: &str) -> Result<UnixStream> {
    let mut delay = Duration::from_millis(10);
    const MAX_DELAY: Duration = Duration::from_secs(5);
    const MAX_ATTEMPTS: u32 = 30;

    for attempt in 1..=MAX_ATTEMPTS {
        match UnixStream::connect(path).await {
            Ok(s) => {
                info!("[si-view] connected to callback socket on attempt {attempt}");
                return Ok(s);
            }
            Err(e) => {
                if attempt == MAX_ATTEMPTS {
                    return Err(e).context(format!("failed to connect to {path}"));
                }
                warn!("[si-view] attempt {attempt}: {e}, retry in {delay:?}");
                sleep(delay).await;
                delay = (delay * 2).min(MAX_DELAY);
            }
        }
    }
    unreachable!()
}
