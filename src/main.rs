use clap::Parser;
use foray::cli::{Cli, Commands};
use foray::server::ForayServer;
use foray::store::JsonFileStore;
use std::sync::Arc;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let store = JsonFileStore::new(JsonFileStore::default_dir()?);

    if matches!(cli.command, Commands::Serve) {
        let server = ForayServer::new(Arc::new(store));
        let transport = rmcp::transport::io::stdio();
        let service = rmcp::serve_server(server, transport).await?;
        service.waiting().await?;
        return Ok(());
    }

    foray::cli::run(&cli, &store)?;
    Ok(())
}
