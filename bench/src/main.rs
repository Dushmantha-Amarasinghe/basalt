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
//! Typical run:
//!
//! ```text
//! # locally, no second machine needed
//! basalt-bench compress
//!
//! # on the laptop
//! basalt-bench gen-corpus --root D:\bench-corpus
//! basalt-bench serve --root D:\bench-corpus
//!
//! # on the PC
//! basalt-bench net --host 192.168.1.42
//! ```
//!
//! Still to build: `disk` (the HDD seek-thrash curve) and `smb` (the baseline
//! the Phase 0 gate is actually measured against).

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use basalt_bench::corpus::{Corpus, CorpusSpec};
use basalt_bench::report::Report;
use basalt_bench::stats::Suite;
use basalt_bench::{compress, net, report};

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
