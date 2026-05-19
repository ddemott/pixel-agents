mod agents;
mod app;
mod caps;
mod chrome;
mod daemon;
mod focus;
mod input_queue;
mod keymap;
mod office;
mod raw_mode;
mod reconnect;
mod render;
mod tui;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let (caps, _pre_app_bytes) = caps::detect().await?;
    app::run(caps).await
}
