use anyhow::Result;
use clap::{Parser, Subcommand};

mod commands;

#[derive(Parser)]
#[command(name = "thane")]
#[command(about = "Control a running thane instance via socket API")]
#[command(version)]
struct Cli {
    /// Path to the Unix domain socket.
    #[arg(long, env = "THANE_SOCKET_PATH")]
    socket: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Check if thane is running.
    Ping,

    /// Workspace management.
    #[command(subcommand)]
    Workspace(commands::workspace::WorkspaceCommand),

    /// Surface/pane management.
    #[command(subcommand)]
    Surface(commands::surface::SurfaceCommand),

    /// Notification management.
    #[command(subcommand)]
    Notification(commands::notification::NotificationCommand),

    /// Sidebar control.
    #[command(subcommand)]
    Sidebar(commands::sidebar::SidebarCommand),

    /// Browser control.
    #[command(subcommand)]
    Browser(commands::browser::BrowserCommand),

    /// Terminal control.
    #[command(subcommand)]
    Terminal(commands::terminal::TerminalCommand),

    /// Sandbox management.
    #[command(subcommand)]
    Sandbox(commands::sandbox::SandboxCommand),

    /// Audit trail management.
    #[command(subcommand)]
    Audit(commands::audit::AuditCommand),

    /// Agent queue management.
    #[command(subcommand)]
    Queue(commands::queue::QueueCommand),

    /// System commands.
    #[command(subcommand)]
    System(commands::system::SystemCommand),
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    let socket_path = cli.socket
        .filter(|s| !s.is_empty())
        .unwrap_or_else(default_socket_path);

    match cli.command {
        Commands::Ping => commands::system::ping(&socket_path).await,
        Commands::Workspace(cmd) => cmd.execute(&socket_path).await,
        Commands::Surface(cmd) => cmd.execute(&socket_path).await,
        Commands::Notification(cmd) => cmd.execute(&socket_path).await,
        Commands::Sidebar(cmd) => cmd.execute(&socket_path).await,
        Commands::Browser(cmd) => cmd.execute(&socket_path).await,
        Commands::Terminal(cmd) => cmd.execute(&socket_path).await,
        Commands::Sandbox(cmd) => cmd.execute(&socket_path).await,
        Commands::Audit(cmd) => cmd.execute(&socket_path).await,
        Commands::Queue(cmd) => cmd.execute(&socket_path).await,
        Commands::System(cmd) => cmd.execute(&socket_path).await,
    }
}

fn default_socket_path() -> String {
    // On macOS, the socket lives under ~/Library/Application Support/thane/run/
    // On Linux, it lives under $XDG_RUNTIME_DIR/thane/ or /tmp/thane-<uid>/thane/
    #[cfg(target_os = "macos")]
    {
        if let Some(app_support) = dirs::data_dir() {
            return app_support
                .join("thane")
                .join("run")
                .join("thane.sock")
                .to_string_lossy()
                .to_string();
        }
    }

    let runtime_dir = dirs::runtime_dir()
        .unwrap_or_else(|| {
            let uid = nix::unistd::getuid();
            std::path::PathBuf::from(format!("/tmp/thane-{uid}"))
        });
    runtime_dir
        .join("thane")
        .join("thane.sock")
        .to_string_lossy()
        .to_string()
}
