mod caps;
mod daemon;
mod input_queue;
mod raw_mode;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let (caps, _pre_app_bytes) = caps::detect().await?;
    daemon::connect(caps).await?;
    Ok(())
}
