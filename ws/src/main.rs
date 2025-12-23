use anyhow::Result;
use backup_sync_ws::server::{ServerConfig, run_server};

#[tokio::main]
async fn main() -> Result<()> {
    let config = ServerConfig::default();
    run_server(config, None).await
}
