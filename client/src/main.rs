mod daemon;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    daemon::connect().await?;
    Ok(())
}
