use clap::Parser;
use foray::cli::{Cli, Commands, resolve_store};
use foray::config::StoreRegistry;
use foray::server::ForayServer;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let registry = StoreRegistry::load()?;

    if matches!(cli.command, Commands::Serve) {
        let server = ForayServer::new(registry);
        let transport = rmcp::transport::io::stdio();
        let service = rmcp::serve_server(server, transport).await?;
        service.waiting().await?;
        return Ok(());
    }

    let store = resolve_store(&registry, cli.store.as_deref())?;
    foray::cli::run(&cli, store).await?;
    Ok(())
}
