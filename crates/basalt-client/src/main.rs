//! `basalt` — the client, without a window.
//!
//! The desktop app and this binary drive exactly the same [`Basalt`] object, so
//! anything that works here works there. That makes it the honest way to check
//! a real host over a real network: no interface in the way, and every failure
//! printed rather than turned into a banner.
//!
//! It is also the right tool when something is wrong. "Does the host answer at
//! all" is a question a graphical app answers badly.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use basalt_client::client::Progress;
use basalt_client::{Basalt, store};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "basalt",
    about = "Talk to a Basalt host from the command line",
    version
)]
struct Cli {
    /// Where paired hosts and their tokens are kept.
    #[arg(long, global = true)]
    store: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Ask a host what it is, without pairing.
    Probe { address: String },

    /// Pair with a host using the PIN on its screen.
    Pair { address: String, pin: String },

    /// List a folder.
    Ls {
        #[arg(default_value = "")]
        path: String,
    },

    /// Download a file.
    Get { remote: String, local: PathBuf },

    /// Upload a file.
    Put {
        local: PathBuf,
        remote: String,
        #[arg(long)]
        overwrite: bool,
    },

    /// Create a folder.
    Mkdir { path: String },

    /// Delete a file or folder.
    Rm {
        path: String,
        #[arg(long, short)]
        recursive: bool,
    },

    /// Show what this device is paired with.
    Status,

    /// Unpair from a host on this side.
    Forget { host_id: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let store_path = cli.store.unwrap_or_else(store::default_path);
    let client = Arc::new(
        Basalt::open(store_path.clone())
            .with_context(|| format!("opening the client store at {}", store_path.display()))?,
    );

    match cli.command {
        Command::Probe { address } => {
            let hello = client.probe(&address).await?;
            println!("host      {}", hello.host_name);
            println!("vault     {}", hello.vault);
            println!("identity  {}", hello.host_id);
            println!(
                "pairing   {}",
                if hello.pairing_open { "open" } else { "closed" }
            );
        }

        Command::Pair { address, pin } => {
            let info = client.pair(&address, &pin).await?;
            println!("paired with {} ({})", info.host_name, info.vault);
            println!("identity {}", info.host_id);
        }

        Command::Status => {
            match client.status() {
                Some(info) => println!(
                    "connected to {} ({}) at {}",
                    info.host_name, info.vault, info.address
                ),
                None => println!("not connected"),
            }
            for host in client.known_hosts() {
                println!(
                    "known  {}  {}  {}",
                    &host.host_id[..8.min(host.host_id.len())],
                    host.host_name,
                    host.last_address.as_deref().unwrap_or("address unknown")
                );
            }
        }

        Command::Forget { host_id } => {
            client.forget(&host_id).await?;
            println!("forgotten");
        }

        // Everything below needs a connection, so it is established once here
        // rather than in each arm.
        other => {
            client
                .connect_saved()
                .await
                .context("connecting to the paired host")?;
            run_connected(&client, other).await?;
        }
    }
    Ok(())
}

async fn run_connected(client: &Arc<Basalt>, command: Command) -> Result<()> {
    match command {
        Command::Ls { path } => {
            let entries = client.list(&path).await?;
            let (free, total) = client.space().await.unwrap_or((0, 0));
            for entry in &entries {
                println!(
                    "{}  {:>12}  {}",
                    if entry.kind == basalt_proto::msg::EntryKind::Dir {
                        "d"
                    } else {
                        "-"
                    },
                    if entry.kind == basalt_proto::msg::EntryKind::Dir {
                        "-".to_string()
                    } else {
                        human(entry.size)
                    },
                    entry.name
                );
            }
            println!(
                "\n{} entries · {} free of {}",
                entries.len(),
                human(free),
                human(total)
            );
        }

        Command::Get { remote, local } => {
            let start = std::time::Instant::now();
            let bytes = client
                .download(&remote, &local, Some(bar("down")), None)
                .await?;
            done(bytes, start);
        }

        Command::Put {
            local,
            remote,
            overwrite,
        } => {
            let start = std::time::Instant::now();
            let bytes = client
                .upload(&local, &remote, overwrite, Some(bar("up")), None)
                .await?;
            done(bytes, start);
        }

        Command::Mkdir { path } => {
            client.mkdir(&path).await?;
            println!("created {path}");
        }

        Command::Rm { path, recursive } => {
            client.remove(&path, recursive).await?;
            println!("removed {path}");
        }

        _ => unreachable!("handled before connecting"),
    }
    Ok(())
}

/// A one-line progress display that rewrites itself.
fn bar(label: &'static str) -> basalt_client::client::ProgressFn {
    use std::io::Write;
    Arc::new(move |p: Progress| {
        let percent = if p.total > 0 {
            (p.transferred as f64 / p.total as f64) * 100.0
        } else {
            100.0
        };
        print!(
            "\r{label} {percent:5.1}%  {} / {}   ",
            human(p.transferred),
            human(p.total)
        );
        let _ = std::io::stdout().flush();
    })
}

fn done(bytes: u64, start: std::time::Instant) {
    let seconds = start.elapsed().as_secs_f64();
    let rate = if seconds > 0.0 {
        bytes as f64 / seconds / 1e6
    } else {
        0.0
    };
    println!(
        "\r{} in {seconds:.1}s  ({rate:.1} MB/s)      ",
        human(bytes)
    );
}

fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
