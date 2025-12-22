use anyhow::Result;
use backup_sync_ws::server::{run_server, ServerConfig};

#[tokio::main]
async fn main() -> Result<()> {
    let config = ServerConfig::default();
    run_server(config, None).await
}
