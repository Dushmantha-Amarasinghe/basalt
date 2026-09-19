//! Asks a running host what a real client sees, and prints it.
//!
//! Built because reasoning about "the client shows an empty folder" from the
//! outside kept producing plausible answers that were wrong. This pairs a
//! genuine client with a genuine host over the real network stack and reports
//! the listing and the library exactly as they arrive — which turned a day of
//! inference into one run.
//!
//! ```text
//! cargo run -p basalt-client --example probe            # 127.0.0.1:7742
//! cargo run -p basalt-client --example probe 192.168.1.90:7742 481920
//! ```
//!
//! The second argument is the PIN the host is displaying, when it asks for one.
//! It pairs as a new device each time, into a throwaway store, so it never
//! disturbs the real client's saved hosts — expect a new entry in the host's
//! device list.

use std::sync::Arc;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let address = args.next().unwrap_or_else(|| "127.0.0.1:7742".into());
    let pin = args.next();

    let addr: std::net::SocketAddr = address.parse().expect("an address like 127.0.0.1:7742");
    let store = std::env::temp_dir().join("basalt-probe-store.json");
    let _ = std::fs::remove_file(&store);

    let client = Arc::new(basalt_client::Basalt::open(store).expect("a fresh store"));
    let requires_pin = client.begin_pairing(addr).await.expect("pairing opens");
    if requires_pin && pin.is_none() {
        eprintln!(
            "that host is asking for a PIN; pass the one it is showing as the second argument"
        );
        return;
    }
    let info = client
        .finish_pairing(pin.as_deref())
        .await
        .expect("pairing completes");
    println!("paired with {:?}", info.vault);

    let entries = client.list("").await.expect("the root lists");
    println!("\nROOT: {} entries", entries.len());
    for entry in entries.iter().take(40) {
        println!("  [{:?}] {}", entry.kind, entry.name);
    }

    // A scan of a whole drive takes a while, and asking before it finishes
    // reports nothing rather than not-yet — which reads as a broken library.
    for _ in 0..90 {
        let library = client.library(0).await.expect("the library answers");
        if library.scanning {
            println!("scanning…");
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            continue;
        }

        let items = library.items.unwrap_or_default();
        println!(
            "\nLIBRARY: {} items (revision {})",
            items.len(),
            library.revision
        );
        for item in &items {
            let episodes: usize = item.seasons.iter().map(|s| s.episodes.len()).sum();
            let where_from = item.path.clone().unwrap_or_else(|| {
                item.seasons
                    .first()
                    .and_then(|s| s.episodes.first())
                    .map(|e| e.path.clone())
                    .unwrap_or_default()
            });
            println!(
                "  {:?} conf {:>3}  {:?} {:?}  {} seasons {} eps  <- {}",
                item.kind,
                item.confidence,
                item.title,
                item.year,
                item.seasons.len(),
                episodes,
                where_from
            );
        }
        return;
    }
    println!("the scan never finished");
}
