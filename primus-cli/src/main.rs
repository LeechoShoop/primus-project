mod admin;
mod commands;
mod config;
mod utils;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "primus", about = "CLI for Primus-Project Layer-1 Network")]
struct Cli {
    /// primus-core node host address.
    /// Override for remote node access.
    #[arg(long, default_value = "127.0.0.1", env = "PRIMUS_NODE_HOST")]
    pub node_host: String,

    /// primus-core node port.
    #[arg(long, default_value = "9000", env = "PRIMUS_NODE_PORT")]
    pub node_port: u16,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Wallet management (create, show)
    Wallet {
        #[command(subcommand)]
        action: WalletAction,
    },
    /// Check balance of an address
    Balance {
        /// Address to check (hex)
        address: String,
        /// Request and verify Merkle proof of balance
        #[arg(long)]
        prove: bool,
    },
    /// Send mass to another address
    Send {
        /// Recipient address (hex)
        #[arg(long)]
        to: String,
        /// Amount of mass to transfer
        #[arg(long)]
        amount: u64,
        /// Path to your wallet file
        #[arg(long, default_value = "my.wallet")]
        from: String,
    },
    /// Administrative commands (requires architect key)
    Admin {
        #[command(subcommand)]
        action: AdminAction,
    },
}

#[derive(Subcommand)]
pub enum AdminAction {
    /// Shutdown the node
    Shutdown {
        #[arg(long, default_value = "my.wallet")]
        wallet: String,
    },
    /// Connect to a peer
    ConnectPeer {
        #[arg(long)]
        addr: String,
        #[arg(long, default_value = "my.wallet")]
        wallet: String,
    },
    /// Node status
    Status,
}

#[derive(Subcommand)]
enum WalletAction {
    /// Create a new 24-word ML-DSA-87 wallet
    Create {
        #[arg(long, default_value = "my.wallet")] // Добавили long
        path: String,
    },
    /// Show address and info for an existing wallet
    Show {
        #[arg(long, default_value = "my.wallet")] // Добавили long
        path: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Wallet { action } => match action {
            WalletAction::Create { path } => commands::wallet::create(path)?,
            WalletAction::Show { path } => commands::wallet::show(path)?,
        },
        Commands::Balance { address, prove } => {
            commands::tx::balance(address, prove, cli.node_host, cli.node_port).await?;
        }
        Commands::Send { to, amount, from } => {
            // Передаем управление в модуль tx
            commands::tx::send(to, amount, from, cli.node_host, cli.node_port).await?;
        }
        Commands::Admin { action } => match action {
            AdminAction::Shutdown { wallet } => admin::shutdown(wallet, cli.node_port).await?,
            AdminAction::ConnectPeer { addr, wallet } => admin::connect_peer(addr, wallet, cli.node_port).await?,
            AdminAction::Status => admin::status(cli.node_port).await?,
        },
    }
    Ok(())
}
