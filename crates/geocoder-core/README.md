# geocoder-core

[![crates.io](https://img.shields.io/crates/v/geocoder-core.svg)](https://crates.io/crates/geocoder-core)
[![docs.rs](https://img.shields.io/docsrs/geocoder-core)](https://docs.rs/geocoder-core)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/Creatiwity/geocoder-core/blob/main/LICENSE)

**Address geocoding and autocomplete in Rust, embedded in your process.**
A faithful port of [addok](https://github.com/addok/addok), the engine
behind the French national address API, with a memory-mapped index instead
of Redis.

- 🎯 **Same answers as the BAN API**: addok 1.3 + addok-fr + addok-france,
  validated function by function against the Python code.
- ⚡ **Fast and light**: about 3 ms per query; the index is memory mapped, so
  serving France, Luxembourg and Belgium takes ~60 MB of private memory.
- 🌍 **Country profiles**: France (BAN), Luxembourg (BD-Adresses), Belgium
  (BeST Address, in French, Dutch and German).
- 🧩 **No server**: build an index file, open it, search. The HTTP API
  ([`geocoder-api`](https://github.com/Creatiwity/geocoder-core#geocoder-api))
  is a separate binary built on this crate.

## Quick start

```toml
[dependencies]
geocoder-core = { version = "0.2", default-features = false }
```

```rust
use geocoder_core::{Geocoder, SearchOpts, index, profile, readers::Source};

// Build once from the BAN export (written next to the target, then renamed;
// flate2 is only needed by this example).
let file = std::fs::File::open("adresses-addok-france.ndjson.gz")?;
let reader = std::io::BufReader::new(flate2::read::MultiGzDecoder::new(file));
index::import(reader, Source::Addok, profile::by_name("fr").unwrap(), "fr.bin".as_ref(), |_| {})?;

// Open (memory mapped) and query.
let geocoder = Geocoder::open("fr.bin".as_ref())?;
for result in geocoder.search("42 rue de rivoli paris", SearchOpts::default()) {
    println!("{} ({}, {}) {:.2}", result.label, result.lat, result.lon, result.score);
}
let nearest = geocoder.reverse(48.8566, 2.3522, Default::default());
```

`Geocoder` is `Send + Sync`: share it behind an `Arc` and query from any
thread.

## Countries

| Profile | Source | Licence | Documents |
|---|---|---|---|
| `fr` | [BAN](https://adresse.data.gouv.fr/) addok export | Etalab 2.0 | 2.5 M (3.3 GB index) |
| `lu` | [BD-Adresses](https://data.public.lu/fr/datasets/adresses-georeferencees-bd-adresses/) | CC0 | 9 k |
| `be` | [BeST Address](https://opendata.bosa.be/) (3 regions) | CC BY 4.0 | 147 k |

`Source::Bano` reads BANO files and `profile::by_name("default")` gives plain
addok, a starting point for another country: a `Profile` overrides only what
differs (query cleanup, housenumbers, synonyms, phonetics, labels, filters).

The profile name and version are stored in the index: opening an index built
by an incompatible version fails instead of returning wrong results.

## Building indexes

Building takes memory: about 5 GB and 6 minutes for France, 4 GB for
Belgium, seconds for Luxembourg. Searching does not. The `cli` feature (on
by default) adds two binaries:

```bash
cargo install geocoder-core
geocoder-build --source addok --profile fr \
    https://adresse.data.gouv.fr/data/ban/adresses/latest/addok/adresses-addok-france.ndjson.gz fr.bin
geocoder-search fr.bin 8 bd du port nanterre
```

## Tracing

Searches, reverse lookups, index opening and builds run in
[`tracing`](https://docs.rs/tracing) spans (INFO): `geocoder.search` (with
`geocoder.collect` and `geocoder.rank`), `geocoder.reverse`,
`geocoder.open`, `geocoder.import` (with `geocoder.write`). Their fields
describe a query's shape, never its text: token counts, the collector that
settled it, the documents scored and the time spent scoring them, the
results. Nothing is recorded without a subscriber; with Sentry's or
OpenTelemetry's, they show up under the caller's span. Calling from
`spawn_blocking`, carry that span (and the Sentry hub) into the closure,
or they start traces of their own.

## Licence

MIT. Ports addok, addok-fr and addok-france (MIT, Etalab / DINUM). The
address data keep their own licences: credit their producers.
