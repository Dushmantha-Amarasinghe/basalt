//! Basalt Phase 0 measurement harness.
//!
//! Phase 0 exists to answer one question before a single line of UI is written:
//! **does a custom protocol actually beat SMB on this link, for this drive?**
//!
//! Building a NAS client is months of work. Discovering in month six that
//! Windows file sharing was already faster would be an expensive way to learn
//! it. So the harness measures the real stack — the same framing and codec code
//! the shipped apps will use — and writes a durable report.
//!
//! Two commands, one per machine:
//!
//! ```text
//! # on the laptop (the machine being measured)
//! basalt-bench host --root D:\bench-corpus
//!
//! # on the other PC — the laptop prints this line with the address filled in
//! basalt-bench measure --host 192.168.1.42
//! ```
//!
//! `host` generates the test files, opens the firewall, creates the Windows
//! share the comparison needs, and then serves. `measure` runs every
//! measurement and ends with a plain-English recommendation.
//!
//! The individual commands (`compress`, `disk`, `net`, `smb`, `gen-corpus`,
//! `serve`) remain available for running one piece at a time.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use basalt_bench::corpus::{Corpus, CorpusSpec};
use basalt_bench::report::Report;
use basalt_bench::stats::Suite;
use basalt_bench::{compress, disk, lab, net, report, smb};

#[derive(Parser)]
#[command(
    name = "basalt-bench",
    about = "Phase 0 measurement harness for Basalt",
    version
)]
struct Cli {
    /// Where to write benchmarks.md and benchmarks.json.
    #[arg(long, global = true, default_value = "docs")]
    out: PathBuf,

    /// Seed for corpus generation. Same seed gives byte-identical files.
    #[arg(long, global = true, default_value_t = 0xBA5A17)]
    seed: u64,

    /// Repetitions per measurement. Medians need at least 5 to mean anything.
    #[arg(long, global = true, default_value_t = 5)]
    runs: usize,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// RUN THIS ON THE LAPTOP. Sets everything up, then waits.
    ///
    /// Generates the test files if they are not already there, opens the
    /// firewall, creates the Windows share used for the comparison, then starts
    /// serving and prints the single command to run on the other PC.
    Host {
        /// Where to put the test files. Point this at the drive being shared.
        #[arg(long, default_value = "bench-corpus")]
        root: PathBuf,

        #[arg(long, default_value_t = net::DEFAULT_PORT)]
        port: u16,

        /// Use a smaller set of test files. Faster to generate, slightly less
        /// reliable numbers.
        #[arg(long)]
        quick: bool,
    },

    /// RUN THIS ON YOUR PC. Does every measurement and prints the answer.
    ///
    /// Needs `basalt-bench host` already running on the laptop.
    Measure {
        /// The address the laptop printed.
        #[arg(long)]
        host: String,

        #[arg(long, default_value_t = net::DEFAULT_PORT)]
        port: u16,

        /// Skip the Windows-sharing comparison. Only use this if the share
        /// cannot be reached — without it there is nothing to compare against.
        #[arg(long)]
        skip_smb: bool,

        /// Override where the Windows share is. Defaults to the share `host`
        /// creates on the laptop.
        #[arg(long)]
        share: Option<PathBuf>,

        /// Bytes per transfer measurement.
        #[arg(long, default_value_t = 256 * 1024 * 1024)]
        transfer_bytes: u64,

        /// How many small files to use.
        #[arg(long, default_value_t = 2000)]
        small_files: usize,
    },

    /// Undo what `host` set up: removes the share and firewall rule.
    Cleanup,

    /// Generate the test corpus.
    GenCorpus {
        #[arg(long, default_value = "bench-corpus")]
        root: PathBuf,

        /// Small corpus for smoke-testing the harness itself.
        #[arg(long)]
        quick: bool,

        /// Regenerate even if a corpus is already present.
        #[arg(long)]
        force: bool,
    },

    /// Compression benchmarks. Runs locally; needs no corpus and no network.
    ///
    /// This is the highest-value measurement in Phase 0 and the one to run
    /// first: on a ~30 MB/s link, zstd runs roughly 20x faster than the radio,
    /// so compressible data should transfer several times faster than raw.
    Compress,

    /// Serve the corpus for network benchmarks. Run this on the laptop.
    Serve {
        /// Directory to serve — point this at the drive under test.
        #[arg(long, default_value = "bench-corpus")]
        root: PathBuf,

        /// Plaintext port. TLS listens on this port plus one.
        #[arg(long, default_value_t = net::DEFAULT_PORT)]
        port: u16,
    },

    /// Network benchmarks. Run this on the PC, against a running `serve`.
    Net {
        /// Hostname or IP of the machine running `serve`.
        #[arg(long)]
        host: String,

        #[arg(long, default_value_t = net::DEFAULT_PORT)]
        port: u16,

        /// Bytes per throughput measurement. Larger is more accurate and
        /// slower; the default takes roughly 10 s per run on Wi-Fi.
        #[arg(long, default_value_t = 256 * 1024 * 1024)]
        transfer_bytes: u64,

        /// How many small files to use in the batch comparison.
        #[arg(long, default_value_t = 2000)]
        small_files: usize,
    },

    /// Disk benchmarks. Runs locally against a generated corpus.
    ///
    /// The important output is the concurrency curve: it decides whether the
    /// host needs an I/O scheduler that caps disk parallelism, or whether a
    /// simple bounded queue is enough.
    Disk {
        /// Corpus directory — point this at the drive under test.
        #[arg(long, default_value = "bench-corpus")]
        root: PathBuf,

        /// Bytes to read per sequential measurement.
        #[arg(long, default_value_t = 512 * 1024 * 1024)]
        sequential_bytes: u64,

        /// Small files to use for the concurrency curve.
        #[arg(long, default_value_t = 2000)]
        small_files: usize,
    },

    /// SMB baseline. Run on the PC against a share on the laptop.
    ///
    /// This is what the Phase 0 gate is measured against.
    Smb {
        /// UNC path to the shared corpus on the laptop.
        #[arg(long)]
        share: PathBuf,

        /// Small files to read. Match your `net --small-files` for a fair
        /// comparison.
        #[arg(long, default_value_t = 2000)]
        small_files: usize,

        /// Bytes to read from the large file.
        #[arg(long, default_value_t = 256 * 1024 * 1024)]
        large_bytes: u64,
    },

    /// Transport lab: find the fastest possible way to move bytes on this link.
    ///
    /// Sweeps TCP buffer and write sizes, then blasts UDP to find the link's
    /// true ceiling. Answers whether any faster transport exists at all.
    Lab {
        /// Address of the machine running `host`.
        #[arg(long)]
        host: String,

        #[arg(long, default_value_t = net::DEFAULT_PORT)]
        port: u16,

        /// Bytes per TCP measurement. Smaller than `measure` because the lab
        /// runs many more of them.
        #[arg(long, default_value_t = 128 * 1024 * 1024)]
        transfer_bytes: u64,

        /// Seconds to blast for each UDP rate.
        #[arg(long, default_value_t = 4.0)]
        udp_seconds: f64,
    },

    /// Show what this machine looks like, including the Wi-Fi link.
    Env,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .without_time()
        .with_target(false)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Host { root, port, quick } => {
            run_host(root, port, quick, cli.seed)?;
        }

        Command::Measure {
            ref host,
            port,
            skip_smb,
            ref share,
            transfer_bytes,
            small_files,
        } => {
            run_measure(
                &cli,
                host,
                port,
                skip_smb,
                share.clone(),
                transfer_bytes,
                small_files,
            )?;
        }

        Command::Cleanup => basalt_bench::setup::cleanup()?,

        Command::GenCorpus { root, quick, force } => {
            let spec = if quick {
                CorpusSpec {
                    seed: cli.seed,
                    ..CorpusSpec::quick()
                }
            } else {
                CorpusSpec {
                    seed: cli.seed,
                    ..CorpusSpec::full()
                }
            };
            Corpus::open(root).generate(spec, force)?;
        }

        Command::Compress => {
            print_banner("compression");
            compress::verify_round_trip(cli.seed)?;

            let mut suites: Vec<Suite> = compress::run(cli.seed, cli.runs)?;

            let mut batching = Suite::new(
                "shared-window-batching",
                "compressing a batch through one zstd context vs one per file",
            );
            for (count, size) in [(1_000usize, 4 * 1024usize), (10_000, 20 * 1024)] {
                batching.push(compress::shared_window_gain(cli.seed, count, size)?);
            }
            suites.push(batching);

            Report::new(suites).write(&cli.out)?;
            print_compression_verdict();
        }

        Command::Serve { root, port } => {
            let (addr, tls_addr) = net::client::resolve_addrs("0.0.0.0", port)?;
            tokio_runtime()?.block_on(net::server::run(net::server::ServerConfig {
                root,
                addr,
                tls_addr,
            }))?;
        }

        Command::Net {
            host,
            port,
            transfer_bytes,
            small_files,
        } => {
            print_banner("network");
            let suites =
                tokio_runtime()?.block_on(net::client::run(net::client::ClientConfig {
                    host,
                    port,
                    tls_port: net::client::default_tls_port(port),
                    runs: cli.runs,
                    transfer_bytes,
                    small_file_count: small_files,
                }))?;
            Report::new(suites).write(&cli.out)?;
            print_network_verdict();
        }

        Command::Disk {
            root,
            sequential_bytes,
            small_files,
        } => {
            print_banner("disk");
            let suites = disk::run(&disk::DiskConfig {
                root,
                runs: cli.runs,
                sequential_bytes,
                small_files,
            })?;
            Report::new(suites).write(&cli.out)?;
        }

        Command::Smb {
            share,
            small_files,
            large_bytes,
        } => {
            print_banner("smb baseline");
            let suites = smb::run(&smb::SmbConfig {
                share,
                runs: cli.runs,
                small_files,
                large_bytes,
            })?;
            Report::new(suites).write(&cli.out)?;
            smb::print_comparison_hint();
        }

        Command::Lab {
            ref host,
            port,
            transfer_bytes,
            udp_seconds,
        } => {
            print_banner("transport lab");
            let suites = tokio_runtime()?.block_on(lab::run(&lab::LabConfig {
                host: host.clone(),
                port,
                runs: cli.runs.min(3),
                transfer_bytes,
                udp_seconds,
            }))?;
            Report::new(suites).write(&cli.out)?;
        }

        Command::Env => {
            let env = report::Environment::detect();
            println!("{}", serde_json::to_string_pretty(&env)?);
            if let Some(advice) = env.wifi.as_ref().and_then(|w| w.advice()) {
                println!("\nadvice: {advice}");
            }
        }
    }

    Ok(())
}

/// The laptop side: set everything up, then serve.
fn run_host(root: PathBuf, port: u16, quick: bool, seed: u64) -> Result<()> {
    use basalt_bench::setup;

    println!("\n┌─ Basalt benchmark host");
    println!("└─ this machine will be measured. leave it running.\n");

    // 1. Test files.
    let spec = if quick {
        CorpusSpec {
            seed,
            ..CorpusSpec::quick()
        }
    } else {
        CorpusSpec {
            seed,
            ..CorpusSpec::full()
        }
    };
    let corpus = Corpus::open(&root);
    if !corpus.is_generated() {
        println!("Creating test files (this happens once, and can take a while):");
    }
    corpus.generate(spec, false)?;

    let absolute = root.canonicalize()?;
    // Strip the \\?\ prefix so the path is usable in a PowerShell command.
    let display_path = absolute
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_string();

    // 2. Firewall and share, so the other PC can actually reach this one.
    println!("\nSetting up access:");
    let fw = setup::ensure_firewall(port);
    println!("{}", fw.describe("Firewall"));
    let share = setup::ensure_share(&display_path);
    println!("{}", share.describe("Windows share"));

    if !fw.is_ok() || !share.is_ok() {
        println!(
            "\n  Some setup needs administrator rights. Close this, right-click\n\
             \x20 PowerShell -> 'Run as administrator', and start it again from\n\
             \x20 there. Everything else will still work, but the comparison\n\
             \x20 against Windows sharing needs the share to exist."
        );
    }

    // 3. Tell the user exactly what to type on the other machine.
    let addresses = setup::local_addresses();
    println!("\n{}", "=".repeat(64));
    match addresses.first() {
        Some(ip) => {
            println!("  On your OTHER PC, run this one command:\n");
            println!("      basalt-bench measure --host {ip}\n");
            if addresses.len() > 1 {
                println!(
                    "  (if that address does not work, try: {})",
                    addresses[1..].join(", ")
                );
            }
        }
        None => println!(
            "  Could not work out this machine's address. Run `ipconfig` and\n\
             \x20 use the IPv4 address of the Wi-Fi adapter."
        ),
    }
    println!("{}", "=".repeat(64));
    println!("\nServing. Leave this window open. Press Ctrl-C when the other PC finishes.\n");

    let bind_addr = format!("0.0.0.0:{port}").parse()?;
    let tls_addr = format!("0.0.0.0:{}", port + 1).parse()?;

    // Bind and serve directly rather than going through `server::run`, which
    // prints its own banner. Two sets of instructions on one screen — one of
    // them telling the user to go and find their own IP address — is worse
    // than none.
    tokio_runtime()?.block_on(async move {
        let bound = net::server::bind(net::server::ServerConfig {
            root: absolute,
            addr: bind_addr,
            tls_addr,
        })
        .await?;
        net::server::serve(bound).await
    })
}

/// The client side: run everything, then say what it means.
#[allow(clippy::too_many_arguments)]
fn run_measure(
    cli: &Cli,
    host: &str,
    port: u16,
    skip_smb: bool,
    share_override: Option<PathBuf>,
    transfer_bytes: u64,
    small_files: usize,
) -> Result<()> {
    use basalt_bench::{setup, smb, verdict};

    println!("\n┌─ Basalt measurement");
    println!("└─ measuring against {host}. this takes a few minutes.\n");

    // How fast can this machine compress? Cheap, and it runs without the
    // network, so do it first.
    println!("[1/4] checking this PC's compression speed");
    compress::verify_round_trip(cli.seed)?;
    let compress_suites = compress::run(cli.seed, 3)?;

    println!("\n[2/4] measuring the custom protocol over the network");
    let net_suites = tokio_runtime()?.block_on(net::client::run(net::client::ClientConfig {
        host: host.to_string(),
        port,
        tls_port: net::client::default_tls_port(port),
        runs: cli.runs,
        transfer_bytes,
        small_file_count: small_files,
    }))?;

    let mut smb_suites = Vec::new();
    let mut real_share = false;
    if skip_smb {
        println!("\n[3/4] skipping the Windows-sharing comparison (--skip-smb)");
    } else {
        let share = share_override
            .unwrap_or_else(|| PathBuf::from(format!(r"\\{host}\{}", setup::SHARE_NAME)));
        real_share = smb::is_network_share(&share);
        println!(
            "\n[3/4] measuring Windows file sharing at {}",
            share.display()
        );
        match smb::run(&smb::SmbConfig {
            share,
            runs: cli.runs,
            small_files,
            large_bytes: transfer_bytes,
        }) {
            Ok(s) => smb_suites = s,
            // A missing share must not throw away the measurements we already
            // have; report it and carry on to the partial result.
            Err(e) => {
                println!("\n  Could not measure Windows sharing: {e}");
                println!(
                    "  Open \\\\{host}\\{} in Explorer once to authenticate, then\n\
                     \x20 run this again. Without it there is nothing to compare against.",
                    setup::SHARE_NAME
                );
            }
        }
    }

    println!("\n[4/4] writing the report");
    let mut all = compress_suites;
    all.extend(net_suites.iter().cloned());
    all.extend(smb_suites.iter().cloned());
    Report::new(all).write(&cli.out)?;

    // Only compare against a real network share. Comparing against a local
    // folder would pit a network protocol against a local disk read and produce
    // a confident, completely wrong recommendation.
    if real_share {
        let comparisons = verdict::compare(&net_suites, &smb_suites);
        verdict::print(&comparisons);
    } else if !skip_smb {
        println!(
            "\n  No verdict: the share given was a local folder, not a network\n\
             \x20 share, so there is nothing valid to compare against."
        );
    }
    println!(
        "  Full numbers: {}\n",
        cli.out.join("benchmarks.md").display()
    );
    Ok(())
}

fn tokio_runtime() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?)
}

fn print_network_verdict() {
    println!(
        "\nWhat to take from this:\n\
         \n\
           · 'raw throughput' is the ceiling. Nothing built on top can exceed it,\n\
             so compare every other number against it rather than against hope.\n\
           · If N streams beats 1 stream by a wide margin, the multi-stream\n\
             transfer engine is justified. Note the count where it plateaus.\n\
           · If tls tracks plain closely, encryption is free at this link speed\n\
             and there is no argument for an unencrypted mode.\n\
           · The batching speedup is the headline: compare it against the SMB\n\
             small-file baseline before committing to the custom protocol.\n"
    );
}

fn print_banner(what: &str) {
    println!("\n┌─ basalt-bench · {what}");
    println!("└─ close other apps; background I/O will skew these numbers\n");
}

fn print_compression_verdict() {
    println!(
        "\nRead the 'effective throughput' table above. The decision rule:\n\
         \n\
           · If zstd-1 effective throughput at 30 MB/s clearly beats 30 MB/s for\n\
             prose/code/json, compression is worth building. Expect 2-4x.\n\
           · If 'binary' shows ~1.0x and near-zero gain, that is correct — the\n\
             entropy sampler is meant to skip it, not compress it.\n\
           · If compression speed on this CPU falls below ~60 MB/s, the laptop\n\
             is too slow to compress inline and the plan needs revisiting.\n"
    );
}
