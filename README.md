# geocoder-core

Address geocoding in Rust, ported from [addok](https://github.com/addok/addok)
to be embedded in other services (e.g. siren), or served on its own by
`geocoder-api` (France, Luxembourg, Belgium). The reference behaviour is
the BAN API as served by the Géoplateforme (IGN): addok 1.3 + addok-fr
1.1.0 + addok-france 1.2.0 with its address index configuration (see
`SCORING.md`).

## Layout

| Part | Role |
|---|---|
| `crates/geocoder-api` | HTTP API compatible with the BAN API, one index per country (see below) |
| `chart` | Helm chart of `geocoder-api` |
| `crates/geocoder-core/src/search` | Generic engine: addok's `Search` helper, its 13 collectors, result scoring |
| `crates/geocoder-core/src/index` | Memory-mapped index: postings, token pairs, edge ngrams, geohashes, filters, documents |
| `crates/geocoder-core/src/profile` | Country specific processing behind the `Profile` trait |
| `crates/geocoder-core/src/readers` | Data sources converted to addok's document format |
| `crates/geocoder-core/src/text` | Language-agnostic text primitives (ASCII folding, tokenizing, synonyms, ngrams) |

### Profiles

Everything that depends on how addresses are written goes through a
`Profile`: query cleanup, token processing (housenumbers, synonyms,
phonetics), labels, indexed fields and filters. Every method defaults to
core addok's behaviour; a country overrides only what it needs.

- `default`: core addok (no phonetics, no synonyms).
- `fr`: the Géoplateforme configuration (addok-fr phonetics and synonyms,
  addok-france ordinals, housenumber detection, cedex/BP cleanup, labels,
  `MIN_SCORE` 0.3).
- `lu`, `be`: the French processing plus 4-digit postcodes ("L-1741",
  "B-1000"), box numbers ("bte 3", "bus 2"), the housenumber after the
  street, and an ordinal glued only before a postcode, a street type or
  nothing. `be` indexes every official language of the region, adds Dutch
  abbreviations and labels "Rue de la Loi 16, 1000 Bruxelles" in the
  language closest to the query.

The profile name and version are stored in the index; opening an index
built with another version fails, since its tokens would no longer match.

### Sources

- `addok` (default): addok's own format, e.g. the BAN export
  `https://adresse.data.gouv.fr/data/ban/adresses/latest/addok/adresses-addok-france.ndjson.gz`.
- `bano`: BANO files, normalized to the BAN types (`municipality`,
  `street`, `locality`) and `context`.
- `lu`: Luxembourg's [BD-Adresses](https://data.public.lu/fr/datasets/adresses-georeferencees-bd-adresses/)
  (CC0, `addresses.csv`).
- `be`: Belgium's [BeST Address](https://opendata.bosa.be/) (CC BY 4.0, one
  zip per region: `openaddress-bevlg.zip`, `-bewal`, `-bebru`).

`lu` and `be` are registers, one row per address: they are grouped into
street, locality and municipality documents (`readers::grouped`), a
housenumber carrying its own postcode and city when they differ from its
street's. Importance is derived from address counts (log scale).

### Reliability of `lu` and `be`

No reference API to compare with, unlike the BAN: `examples/roundtrip.rs`
samples the register, writes each address several ways and checks that the
first result is that address (within 30 m, housenumber matched).

| Written as | Luxembourg (3,000) | Belgium (4,000) |
|---|---:|---:|
| `12 Rue X 1234 City` | 99.9 % | 99.0 % |
| `Rue X 12, L-1234 City` | 99.9 % | 99.0 % |
| no postcode | 99.9 % | 98.9 % |
| lowercase, no accents | 99.8 % | 98.8 % |
| one letter dropped in the street | 99.9 % | 91.2 % |
| other language (`Wetstraat 16, 1000 Brussel`) | | 100 % (77) |

What is left is mostly genuine ambiguity (two "Machelen", numbers inside
street names) and typos inside Dutch compound words ("Kanaalsraat").

```bash
cargo run --release --example roundtrip -- --source be --sample 4000 --every 500 \
    --failures failures.tsv data/be.bin openaddress-bevlg.zip openaddress-bewal.zip openaddress-bebru.zip
```

## Usage

```bash
cargo build --release
./target/release/geocoder-build --source addok --profile fr \
    data/adresses-addok-france.ndjson.gz data/index-ban.bin
./target/release/geocoder-build --source be \
    https://opendata.bosa.be/download/best/openaddress-bevlg.zip \
    https://opendata.bosa.be/download/best/openaddress-bewal.zip \
    https://opendata.bosa.be/download/best/openaddress-bebru.zip data/be.bin
./target/release/geocoder-search data/index-ban.bin 42 rue de rivoli paris
echo "bd haussmann paris" | ./target/release/geocoder-search --batch data/index-ban.bin

# Trace the collectors, like addok's --verbose
GEOCODER_DEBUG=1 ./target/release/geocoder-search data/index-ban.bin rebenac lag
```

### As a library

```toml
geocoder-core = { git = "https://github.com/Creatiwity/geocoder-core", tag = "v0.2.0", default-features = false }
```

```rust
use geocoder_core::{Geocoder, SearchOpts, index, profile, readers::Source};

// Build (atomic: written next to the target, then renamed).
let reader = std::io::BufReader::new(flate2::read::MultiGzDecoder::new(std::fs::File::open(path)?));
index::import(reader, Source::Addok, profile::by_name("fr").unwrap(), "index.bin".as_ref(), |_| {})?;

// Query.
let geocoder = Geocoder::open("index.bin".as_ref())?;
let results = geocoder.search("42 rue de rivoli paris", SearchOpts::default());
```

The `cli` feature (on by default) only adds the `geocoder-build` and
`geocoder-search` binaries.

## Validation

Two levels, both against addok itself:

1. **Pipeline, function by function** — `scripts/addok_oracle.py` runs the
   real addok code (tokens, index boosts, labels, scores) without Redis;
   `scripts/diff_pipeline.py` compares it with the Rust implementation
   (`crates/geocoder-core/examples/pipeline.rs`) on documents sampled from a BAN export and
   queries derived from them.

   ```bash
   uv venv -p 3.12 .oracle
   VIRTUAL_ENV=.oracle uv pip install addok==1.3.2 addok-fr==1.1.0 addok-france==1.2.0
   cargo build --release --example pipeline
   python3 scripts/diff_pipeline.py --oracle .oracle/bin/python \
       --data data/adresses-addok-france.ndjson.gz --every 97 --max-docs 25000
   ```

2. **End to end** — `scripts/compare_api.py` runs the same queries against
   the BAN API and the Rust index, and reports how often the first result
   matches.

   ```bash
   python3 scripts/compare_api.py data/index-ban.bin \
       --data data/adresses-addok-france.ndjson.gz --sample 200
   ```

## License

MIT. geocoder-core ports addok, addok-fr and addok-france (MIT, Etalab /
DINUM); see `LICENSE`.

## geocoder-api

One process serving one index per country, answering like the BAN API
(`api-adresse.data.gouv.fr`): `GET /search` (`q`, `limit`, `autocomplete`,
`lat`, `lon`, `type`, `postcode`, `citycode`) and `GET /reverse`, GeoJSON
`FeatureCollection` with `attribution` and `licence` per country, plus a
`country` parameter (`fr`, `lu`, `be`).

```bash
export GEOCODER_COUNTRIES=fr,lu,be GEOCODER_DATA_DIR=data
cargo run --release -p geocoder-api -- update        # downloads, builds what changed
cargo run --release -p geocoder-api -- serve --port 8080
curl 'localhost:8080/search?q=wetstraat+16+brussel&country=be'
```

| Variable | Default | |
|---|---|---|
| `GEOCODER_COUNTRIES` | `fr` | Countries served, `<code>.bin` in the data dir |
| `GEOCODER_DEFAULT_COUNTRY` | the first | Without `country` |
| `GEOCODER_DATA_DIR` | `data` (`/data` in the image) | Indexes, sidecars, downloads |
| `GEOCODER_SOURCE_URLS_<CC>` | built in | Comma separated URLs or paths |
| `GEOCODER_RELOAD_INTERVAL_SECONDS` | 60 | Index files checked for a new build |
| `HOST` / `PORT` | `0.0.0.0` / 8080 (3000 in the image) | |
| `SHUTDOWN_DELAY_SECONDS` | 5 | Readiness fails this long after SIGTERM |
| `SENTRY_DSN`, `GEOCODER_ENV`, `RUST_LOG` | | |

Other routes: `/countries` (loaded, documents, sources and build date),
`/health/live`, `/health/ready` (an index loaded once one is built, not
shutting down), `/openapi.json`, `/scalar`.

`update` skips a country whose sources did not change (`Last-Modified` /
`ETag`, kept in `<code>.bin.source.json`), and rebuilds atomically: the
server picks the new file up without a restart.

The Helm chart (`chart/`) follows the siren one: a Deployment reading the
indexes from a volume, an `update` CronJob writing them, optional
ListenerSet / HTTPRoute / auth (kgateway or APISIX) and Ingress. After the
first install, build the indexes once:

```bash
kubectl create job --from=cronjob/<fullname>-update <fullname>-init
```
