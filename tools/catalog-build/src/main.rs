//! Rebuilds `crates/basalt-catalog/data/catalog.bin` from Wikidata.
//!
//! Run once per release, never on anybody's machine but the one building it:
//!
//! ```text
//! cargo run -p catalog-build --release
//! ```
//!
//! **Where the data comes from.** Wikidata, through QLever — the University of
//! Freiburg's SPARQL engine over the same data. Wikidata's own query service
//! took 41 seconds for one year of films in 2026, against a 60-second limit,
//! so a full extraction there means a hundred-odd paged queries any of which
//! may time out. QLever answers the whole thing in under a minute. Another
//! endpoint serving the same data can be named with `--endpoint`.
//!
//! **What is taken, for films and for series alike:** every English label, the
//! multilingual label, English aliases, and the original title — the names a
//! release is likely to be filed under — plus the first and last year each was
//! released, or for a series the years it ran. Nothing else: the host only
//! ever asks "does this exist, and when".
//!
//! **What stops a bad build shipping.** A partial answer from an endpoint looks
//! exactly like a smaller catalogue, and a smaller catalogue quietly removes
//! real films from people's libraries. So the build refuses to write anything
//! unless the counts are in the range a real extraction produces and a handful
//! of well-known titles resolve.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use basalt_catalog::{Builder, Catalog, Kind, Lookup};

const DEFAULT_ENDPOINT: &str = "https://qlever.dev/api/wikidata";
const USER_AGENT: &str = concat!(
    "BasaltCatalogBuilder/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/Dushmantha-Amarasinghe/basalt)"
);

const PREFIXES: &str = "PREFIX wd: <http://www.wikidata.org/entity/>
PREFIX wdt: <http://www.wikidata.org/prop/direct/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
PREFIX skos: <http://www.w3.org/2004/02/skos/core#>
";

/// Films: `film` and everything that is a kind of film — documentaries,
/// television films, animated features.
const FILMS: &str = "?item wdt:P31/wdt:P279* wd:Q11424 .";

/// Series: `television series` and `web series`, and their subclasses —
/// miniseries, anime series, and the rest. Streaming originals are usually
/// filed as one or the other.
const SERIES: &str = "{ ?item wdt:P31/wdt:P279* wd:Q5398426 } \
                      UNION { ?item wdt:P31/wdt:P279* wd:Q526877 }";

/// Below these, the endpoint gave a partial answer and nothing is written.
/// About two thirds of what a 2026 extraction produced.
const MIN_FILM_NAMES: usize = 400_000;
const MIN_SERIES_NAMES: usize = 80_000;

struct Args {
    endpoint: String,
    out: PathBuf,
}

fn args() -> Result<Args> {
    let mut endpoint = DEFAULT_ENDPOINT.to_string();
    let mut out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/basalt-catalog/data/catalog.bin");
    let mut given = std::env::args().skip(1);
    while let Some(arg) = given.next() {
        match arg.as_str() {
            "--endpoint" => endpoint = given.next().context("--endpoint needs a URL")?,
            "--out" => out = given.next().context("--out needs a path")?.into(),
            other => bail!("unknown argument {other}; expected --endpoint or --out"),
        }
    }
    Ok(Args { endpoint, out })
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = args()?;
    basalt_net::tls::init_crypto();
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(600))
        .build()?;

    let mut builder = Builder::new(today());
    let mut counts = HashMap::new();

    for (kind, class, years_from) in [
        (Kind::Film, FILMS, "?item wdt:P577 ?d"),
        (
            Kind::Series,
            SERIES,
            "{ ?item wdt:P580 ?d } UNION { ?item wdt:P577 ?d }",
        ),
    ] {
        let years = fetch_years(&client, &args.endpoint, class, years_from).await?;
        eprintln!("{kind:?}: years for {} items", years.len());

        let mut names = 0usize;
        for (what, pattern) in [
            (
                "labels",
                "?item rdfs:label ?name . FILTER(LANG(?name) = \"en\" || LANG(?name) = \"mul\")",
            ),
            (
                "aliases",
                "?item skos:altLabel ?name . FILTER(LANG(?name) = \"en\")",
            ),
            ("original titles", "?item wdt:P1476 ?name ."),
        ] {
            let query =
                format!("{PREFIXES}SELECT DISTINCT ?item ?name WHERE {{ {class} {pattern} }}");
            let rows = fetch(&client, &args.endpoint, &query).await?;
            eprintln!("{kind:?}: {} {what}", rows.len());
            for row in rows {
                let [item, name] = row.as_slice() else {
                    continue;
                };
                let (Some(item), Some(name)) = (entity(item), literal(name)) else {
                    continue;
                };
                builder.add(kind, &name, years.get(item).copied());
                names += 1;
            }
        }
        counts.insert(kind, names);
    }

    let films = counts[&Kind::Film];
    let series = counts[&Kind::Series];
    if films < MIN_FILM_NAMES || series < MIN_SERIES_NAMES {
        bail!(
            "only {films} film names and {series} series names came back — the endpoint gave a \
             partial answer. Nothing was written."
        );
    }

    let catalog = builder.finish();
    check(&catalog)?;
    let bytes = catalog.encode();
    std::fs::write(&args.out, &bytes)
        .with_context(|| format!("could not write {}", args.out.display()))?;
    eprintln!(
        "wrote {} titles ({films} film names, {series} series names) to {} — {:.1} MB, snapshot {}",
        catalog.len(),
        args.out.display(),
        bytes.len() as f64 / 1e6,
        catalog.snapshot(),
    );
    Ok(())
}

/// Refuses a catalogue that does not know what any real one would.
fn check(catalog: &Catalog) -> Result<()> {
    let must = [
        (Kind::Film, "Arrival", 2016),
        (Kind::Film, "Blade Runner 2049", 2017),
        (Kind::Film, "Spirited Away", 2001),
        (Kind::Film, "Amélie", 2001),
        (Kind::Series, "Breaking Bad", 2008),
        (Kind::Series, "The Office", 2005),
    ];
    for (kind, title, year) in must {
        if catalog.find(kind, title, Some(year)) != Lookup::Found {
            bail!("the new catalogue does not know {title} ({year}); nothing was written");
        }
    }
    Ok(())
}

/// First and last year of release for every item of one kind.
async fn fetch_years(
    client: &reqwest::Client,
    endpoint: &str,
    class: &str,
    dates: &str,
) -> Result<HashMap<String, (u16, u16)>> {
    let query = format!(
        "{PREFIXES}SELECT ?item (MIN(YEAR(?d)) AS ?first) (MAX(YEAR(?d)) AS ?last) \
         WHERE {{ {class} {dates} }} GROUP BY ?item"
    );
    let mut years = HashMap::new();
    for row in fetch(client, endpoint, &query).await? {
        let [item, first, last] = row.as_slice() else {
            continue;
        };
        let (Some(item), Some(first), Some(last)) = (entity(item), year(first), year(last)) else {
            continue;
        };
        years.insert(item.to_string(), (first, last));
    }
    Ok(years)
}

/// Runs one query and returns its rows, header dropped.
async fn fetch(client: &reqwest::Client, endpoint: &str, query: &str) -> Result<Vec<Vec<String>>> {
    let url = reqwest::Url::parse_with_params(endpoint, &[("query", query)])
        .with_context(|| format!("{endpoint} is not a URL"))?;
    let response = client
        .get(url)
        .header("Accept", "text/tab-separated-values")
        .send()
        .await
        .context("the endpoint could not be reached")?;
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        bail!(
            "the endpoint answered {status}: {}",
            text.chars().take(300).collect::<String>()
        );
    }
    Ok(text
        .lines()
        .skip(1)
        .map(|line| line.split('\t').map(str::to_string).collect())
        .collect())
}

/// `<http://www.wikidata.org/entity/Q42>` to `Q42`.
fn entity(cell: &str) -> Option<&str> {
    cell.strip_prefix("<http://www.wikidata.org/entity/")?
        .strip_suffix('>')
}

/// A year cell, however the endpoint chose to type it. Years before the
/// common era are not films and are dropped.
fn year(cell: &str) -> Option<u16> {
    let bare = cell.trim_start_matches('"');
    let digits: String = bare.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() || bare.starts_with('-') {
        return None;
    }
    digits
        .parse()
        .ok()
        .filter(|&y: &u16| (1800..=2200).contains(&y))
}

/// A TSV literal — `"text"@en`, `"text"` or `"text"^^<type>` — to its text.
fn literal(cell: &str) -> Option<String> {
    let rest = cell.strip_prefix('"')?;
    let end = rest.rfind('"')?;
    let mut out = String::with_capacity(end);
    let mut chars = rest[..end].chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            'n' | 'r' | 't' => out.push(' '),
            'u' => out.push(hex_char(&mut chars, 4)?),
            'U' => out.push(hex_char(&mut chars, 8)?),
            other => out.push(other),
        }
    }
    Some(out)
}

fn hex_char(chars: &mut std::str::Chars<'_>, len: usize) -> Option<char> {
    let hex: String = chars.take(len).collect();
    char::from_u32(u32::from_str_radix(&hex, 16).ok()?)
}

/// Today as `yyyymmdd`, which becomes the catalogue's snapshot date.
fn today() -> u32 {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0) as i64;
    // Howard Hinnant's days-to-civil, as the host's build stamp uses.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year * 10_000 + month * 100 + day) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_are_read_however_they_are_typed() {
        assert_eq!(literal("\"Arrival\"@en").as_deref(), Some("Arrival"));
        assert_eq!(literal("\"Tinko\"@de").as_deref(), Some("Tinko"));
        assert_eq!(literal("\"plain\"").as_deref(), Some("plain"));
        assert_eq!(
            literal("\"He said \\\"no\\\"\"@en").as_deref(),
            Some("He said \"no\"")
        );
        assert_eq!(literal("\"Am\\u00E9lie\"@fr").as_deref(), Some("Amélie"));
        assert_eq!(literal("\"a\\tb\"").as_deref(), Some("a b"));
        assert_eq!(literal("no quotes"), None);
    }

    #[test]
    fn entities_and_years_are_read_off_their_cells() {
        assert_eq!(entity("<http://www.wikidata.org/entity/Q42>"), Some("Q42"));
        assert_eq!(entity("Q42"), None);
        assert_eq!(year("1999"), Some(1999));
        assert_eq!(
            year("\"2016\"^^<http://www.w3.org/2001/XMLSchema#int>"),
            Some(2016)
        );
        assert_eq!(year("-0044"), None);
        assert_eq!(year(""), None);
    }

    #[test]
    fn today_is_a_plausible_date() {
        let t = today();
        assert!(t > 20260101 && t < 21000101, "{t}");
        assert!((1..=12).contains(&(t / 100 % 100)));
        assert!((1..=31).contains(&(t % 100)));
    }
}
