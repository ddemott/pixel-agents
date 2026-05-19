mod agents;
mod app;
mod caps;
mod chrome;
mod daemon;
mod focus;
mod input_queue;
mod keymap;
mod raw_mode;
mod reconnect;
mod tui;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let (caps, _pre_app_bytes) = caps::detect().await?;
    app::run(caps).await
}
