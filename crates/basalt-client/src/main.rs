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

    /// Pair with a host, by address.
    ///
    /// The app discovers hosts instead; this is for when something is wrong and
    /// you want to take discovery out of the picture. The PIN is asked for here
    /// rather than passed as an argument — see the command itself for why.
    Pair { address: String },

    /// List every Basalt host answering on this network.
    Find,

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

    /// Stream a file over a local URL any player can open.
    ///
    /// Prints the URL and keeps serving until stopped. Paste it into VLC or
    /// mpv to watch a film off the vault without downloading it first — the
    /// same mechanism the app's own player uses.
    Url { path: String },

    /// Play a file in an installed player, streamed rather than downloaded.
    Play { path: String },

    /// Show which media player the app would hand a file to.
    Player,

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

        Command::Pair { address } => {
            let addr = basalt_net::socket::resolve(&address, basalt_net::DEFAULT_PORT).await?;

            // Asking is what makes the host generate and display the PIN, so
            // the number is only valid for *this* attempt. Reading it from
            // stdin keeps one session open across both steps; taking it as an
            // argument would mean a second run, a second request, and a second
            // PIN — leaving the one on the host's screen already stale.
            let requires_pin = client.begin_pairing(addr).await?;
            let pin = if requires_pin {
                println!("This host is showing a PIN. Type it here:");
                let mut typed = String::new();
                std::io::stdin().read_line(&mut typed)?;
                Some(typed.trim().to_string())
            } else {
                println!("This host does not ask for a PIN.");
                None
            };

            let info = client.finish_pairing(pin.as_deref()).await?;
            println!(
                "
paired with {} ({})",
                info.host_name, info.vault
            );
            println!("identity {}", info.host_id);
        }

        Command::Find => {
            // The same call the app makes, so this command exercises the
            // ordering and the already-paired marking rather than a parallel
            // path that could drift away from them.
            let found = client.discover_hosts().await?;
            if found.is_empty() {
                println!("nothing answered. Is the host running on this network?");
            }
            for host in &found {
                println!(
                    "{}  {}  ({})  {}{}{}",
                    host.address,
                    host.host_name,
                    host.vault,
                    if host.requires_pin {
                        "PIN required"
                    } else {
                        "no PIN"
                    },
                    if host.paired { "  · paired" } else { "" },
                    if host.has_vault {
                        ""
                    } else {
                        "  · no drive shared yet"
                    },
                );
            }
        }

        Command::Player => match basalt_client::players::find() {
            Some(player) => println!("{} at {}", player.name, player.path.display()),
            None => println!(
                "none found. Install VLC or mpv to stream files the window                      cannot decode."
            ),
        },

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

        Command::Url { path } => {
            // Confirm it exists before printing a URL that would 404.
            let entry = client.stat(&path).await?;
            let proxy = basalt_client::proxy::MediaProxy::start(Arc::clone(client)).await?;
            println!("{}", proxy.url_for(&path));
            println!(
                "
{} · {}
open that in VLC or mpv. ctrl-c to stop serving.",
                entry.name,
                human(entry.size)
            );
            // The proxy lives on a background task, so this has to stay alive.
            std::future::pending::<()>().await;
        }

        Command::Play { path } => {
            let entry = client.stat(&path).await?;
            let player = basalt_client::players::find().context(
                "no player found that can open a URL. Install VLC or mpv, or use                  `basalt url` and paste the address in yourself.",
            )?;
            let proxy = basalt_client::proxy::MediaProxy::start(Arc::clone(client)).await?;
            let url = proxy.url_for(&path);

            basalt_client::players::launch(&player, &url)
                .with_context(|| format!("starting {}", player.name))?;
            println!(
                "streaming {} ({}) to {}
nothing is being downloaded. ctrl-c when done.",
                entry.name,
                human(entry.size),
                player.name
            );
            // The proxy lives on a background task, so this has to stay alive
            // for as long as the player is reading from it.
            std::future::pending::<()>().await;
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
