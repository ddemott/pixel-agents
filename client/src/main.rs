mod agents;
mod app;
mod caps;
mod chrome;
mod daemon;
mod focus;
mod input_queue;
mod keymap;
mod raw_mode;
mod tui;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let (caps, _pre_app_bytes) = caps::detect().await?;
    let conn = daemon::connect(caps).await?;
    app::run(conn).await
}
