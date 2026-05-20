use anyhow::Result;

use pixel_agents_tui::{app, caps};

#[tokio::main]
async fn main() -> Result<()> {
    let (caps, _pre_app_bytes) = caps::detect().await?;
    app::run(caps).await
}
