use clap::Parser;
use foray::cli::{Cli, Commands};
use foray::config::StoreRegistry;
use foray::server::ForayServer;
use foray::store_json::JsonFileStore;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if matches!(cli.command, Commands::Serve) {
        let registry = StoreRegistry::load()?;
        let server = ForayServer::new(registry);
        let transport = rmcp::transport::io::stdio();
        let service = rmcp::serve_server(server, transport).await?;
        service.waiting().await?;
        return Ok(());
    }

    let store = JsonFileStore::new(JsonFileStore::default_dir()?);
    foray::cli::run(&cli, &store)?;
    Ok(())
}
