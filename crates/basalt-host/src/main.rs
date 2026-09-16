//! `basalt-host` — the program that runs on the machine with the drive.
//!
//! A console app on purpose for this phase. What has to be right first is that
//! the connection works over a real radio with a real drive behind it; a
//! window around it is worth building once there is something to put in it.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use basalt_host::config::{self, HostConfig};
use basalt_host::server::{self, Host};
use basalt_net::identity::short_id;
use basalt_net::pairing::format_pin;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "basalt-host",
    about = "Share one drive with Basalt clients on this network",
    version
)]
struct Cli {
    /// Where the host keeps its identity and paired devices.
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serve a drive or folder.
    Serve {
        /// The drive or folder to share, for example `E:\`.
        #[arg(long, short)]
        path: Option<PathBuf>,

        /// What clients will call it.
        #[arg(long, short)]
        name: Option<String>,

        #[arg(long, default_value_t = basalt_net::DEFAULT_PORT)]
        port: u16,

        /// Share without allowing any client to modify the drive.
        #[arg(long)]
        read_only: bool,
    },

    /// List paired devices.
    Devices,

    /// Forget a paired device. It will have to pair again.
    Revoke {
        /// The token hash shown by `devices`. A unique prefix is enough.
        device: String,
    },

    /// Show this host's identity.
    Id,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "basalt_host=info".into()),
        )
        .with_target(false)
        .without_time()
        .init();

    let cli = Cli::parse();
    let config_path = cli.config.unwrap_or_else(config::default_path);

    match cli.command {
        Command::Serve {
            path,
            name,
            port,
            read_only,
        } => serve(config_path, path, name, port, read_only).await,
        Command::Devices => devices(config_path),
        Command::Revoke { device } => revoke(config_path, &device),
        Command::Id => id(config_path),
    }
}

fn load(config_path: &std::path::Path) -> Result<HostConfig> {
    HostConfig::load_or_create(config_path, &config::machine_name())
        .with_context(|| format!("reading {}", config_path.display()))
}

async fn serve(
    config_path: PathBuf,
    path: Option<PathBuf>,
    name: Option<String>,
    port: u16,
    read_only: bool,
) -> Result<()> {
    let mut config = load(&config_path)?;
    config.port = port;

    // A path on the command line replaces whatever was remembered, so the user
    // can point the host at a different drive without editing a config file.
    if let Some(path) = path {
        config.vault_path = Some(path);
        if let Some(name) = name.clone() {
            config.vault_name = name;
        }
        config.save(&config_path)?;
    }

    let Some(vault_path) = config.vault_path.clone() else {
        anyhow::bail!(
            "no drive chosen yet. Run:  basalt-host serve --path E:\\ --name \"My Drive\""
        );
    };
    if read_only {
        tracing::info!("serving read-only");
    }

    let host = Host::new(config.clone(), config_path.clone())?;
    let addr: SocketAddr = format!("0.0.0.0:{port}").parse().expect("a valid address");
    let bound = server::bind(Arc::clone(&host), addr).await?;

    banner(&host, &vault_path, bound.addr());

    // A host with nothing paired is useless, so open the window immediately
    // rather than making the user find the command for it.
    if host.devices().is_empty() {
        let pin = host.open_pairing()?;
        println!("  No devices paired yet.");
        println!("  Pairing PIN:  {}\n", format_pin(&pin));
    } else {
        println!("  {} device(s) paired.", host.devices().len());
        println!("  Press Enter to open pairing for another. Ctrl-C to stop.\n");
    }

    // Console input runs alongside the server so a PIN can be reissued without
    // restarting, which matters because a window only lasts three minutes.
    let console = tokio::spawn(console_loop(Arc::clone(&host)));
    let result = server::serve(bound).await;
    console.abort();
    result.map_err(Into::into)
}

/// Reissues a PIN whenever the user presses Enter.
async fn console_loop(host: Arc<Host>) {
    use tokio::io::AsyncBufReadExt;

    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        match line.trim() {
            "d" | "devices" => {
                for device in host.devices() {
                    println!(
                        "  {}  {}  {}",
                        short_id(&device.token_hash),
                        device.name,
                        if device.writable {
                            "read-write"
                        } else {
                            "read-only"
                        }
                    );
                }
                if host.devices().is_empty() {
                    println!("  nothing paired yet");
                }
            }
            _ => match host.open_pairing() {
                Ok(pin) => println!("\n  Pairing PIN:  {}   (3 minutes)\n", format_pin(&pin)),
                Err(e) => println!("  could not open pairing: {e}"),
            },
        }
    }
}

fn banner(host: &Arc<Host>, vault_path: &std::path::Path, addr: SocketAddr) {
    println!("\n  Basalt Host");
    println!("  sharing   {}", vault_path.display());
    println!("  as        {}", host.vault_name());
    println!("  identity  {}", short_id(host.host_id()));
    println!("  port      {}", addr.port());
    for ip in local_addresses() {
        println!("  reachable at  {ip}");
    }
    println!();
}

fn devices(config_path: PathBuf) -> Result<()> {
    let config = load(&config_path)?;
    if config.devices.is_empty() {
        println!("nothing paired yet");
        return Ok(());
    }
    for device in &config.devices {
        println!(
            "{}  {:<24}  {}",
            short_id(&device.token_hash),
            device.name,
            if device.writable {
                "read-write"
            } else {
                "read-only"
            }
        );
    }
    Ok(())
}

fn revoke(config_path: PathBuf, prefix: &str) -> Result<()> {
    let mut config = load(&config_path)?;
    let before = config.devices.len();

    let matches: Vec<String> = config
        .devices
        .iter()
        .filter(|d| d.token_hash.starts_with(prefix))
        .map(|d| d.name.clone())
        .collect();

    // Refusing an ambiguous prefix rather than guessing: revoking the wrong
    // device means someone's laptop silently stops working.
    if matches.len() > 1 {
        anyhow::bail!(
            "{prefix} matches {} devices; be more specific",
            matches.len()
        );
    }

    config.devices.retain(|d| !d.token_hash.starts_with(prefix));
    if config.devices.len() == before {
        anyhow::bail!("no device matches {prefix}");
    }
    config.save(&config_path)?;
    println!(
        "revoked {}",
        matches.first().map(String::as_str).unwrap_or(prefix)
    );
    Ok(())
}

fn id(config_path: PathBuf) -> Result<()> {
    let config = load(&config_path)?;
    println!("{}", config.identity()?.host_id);
    Ok(())
}

/// This machine's LAN addresses, so nobody has to go hunting in `ipconfig`.
fn local_addresses() -> Vec<String> {
    let Ok(out) = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-NetIPAddress -AddressFamily IPv4 | \
             Where-Object { $_.IPAddress -notlike '127.*' -and \
             $_.IPAddress -notlike '169.254.*' }).IPAddress",
        ])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}
