//! Times one upload and one download between a host and a client, in-process.
//!
//! Real TLS over real sockets on this machine, so it measures everything the
//! protocol and the two ends add — hashing, disk, framing, how much waiting
//! each chunk does — without the radio in the way. Most useful as a before and
//! after: a change that makes this faster makes the per-chunk overhead smaller,
//! and that overhead is exactly what a slow link multiplies.
//!
//! ```text
//! cargo run -p basalt-client --example transfer_bench --release          # 1 GiB
//! cargo run -p basalt-client --example transfer_bench --release -- 256   # MiB
//! ```

use std::sync::Arc;
use std::time::Instant;

use basalt_client::Basalt;
use basalt_host::config::HostConfig;
use basalt_host::server::{self, Host};

#[tokio::main]
async fn main() {
    let mib: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(1024);
    let size = mib * 1024 * 1024;

    let dir = std::env::temp_dir().join(format!("basalt-bench-{}", std::process::id()));
    let vault = dir.join("vault");
    std::fs::create_dir_all(&vault).unwrap();

    // Bytes that do not compress to nothing, written once.
    let source = dir.join("source.bin");
    {
        use std::io::Write;
        let mut file = std::io::BufWriter::new(std::fs::File::create(&source).unwrap());
        let block: Vec<u8> = (0..1024 * 1024u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
            .collect();
        for _ in 0..mib {
            file.write_all(&block).unwrap();
        }
    }

    let config_path = dir.join("host.json");
    let mut config = HostConfig::create("bench-host").unwrap();
    config.vault_path = Some(vault.clone());
    config.vault_name = "Bench".into();
    config.require_pin = false;
    config.save(&config_path).unwrap();
    let host = Host::new(config, config_path).unwrap();
    let bound = server::bind(Arc::clone(&host), "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let addr = bound.addr();
    tokio::spawn(server::serve(bound));

    let client = Arc::new(Basalt::open(dir.join("client.json")).unwrap());
    client.pair_with(addr, None).await.expect("pairs");

    let began = Instant::now();
    client
        .upload(&source, "copy.bin", true, None, None)
        .await
        .expect("uploads");
    let up = began.elapsed().as_secs_f64();

    let back = dir.join("back.bin");
    let began = Instant::now();
    client
        .download("copy.bin", &back, None, None)
        .await
        .expect("downloads");
    let down = began.elapsed().as_secs_f64();

    assert_eq!(
        std::fs::metadata(&back).unwrap().len(),
        size,
        "the round trip lost bytes"
    );
    println!(
        "{mib} MiB  upload {:.0} MB/s ({up:.2}s)  download {:.0} MB/s ({down:.2}s)",
        size as f64 / up / 1e6,
        size as f64 / down / 1e6,
    );
    let _ = std::fs::remove_dir_all(&dir);
}
