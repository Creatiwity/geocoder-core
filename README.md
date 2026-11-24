<div align="center">

# geocoder-core

**Address geocoding and autocomplete for France, Luxembourg and Belgium, in Rust.**

A faithful port of [addok](https://github.com/addok/addok), the engine of the
French national address API, as a library and as a ready-to-run HTTP API.

[![crates.io](https://img.shields.io/crates/v/geocoder-core.svg)](https://crates.io/crates/geocoder-core)
[![docs.rs](https://img.shields.io/docsrs/geocoder-core)](https://docs.rs/geocoder-core)
[![CI](https://github.com/Creatiwity/geocoder-core/actions/workflows/ci.yml/badge.svg)](https://github.com/Creatiwity/geocoder-core/actions/workflows/ci.yml)
[![Docker](https://img.shields.io/docker/v/creatiwity/geocoder-api?label=docker&sort=semver)](https://hub.docker.com/r/creatiwity/geocoder-api)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

[Library](#the-library) · [HTTP API](#geocoder-api) · [Deployment](#deployment) · [Reliability](#reliability) · [Validation](#validation)

</div>

---

## Why

- 🎯 **The BAN API's answers, without the BAN API.** The ranking reproduces
  addok 1.3 + addok-fr 1.1 + addok-france 1.2 with the Géoplateforme (IGN)
  configuration, checked function by function against the Python code
  ([`SCORING.md`](SCORING.md)).
- ⚡ **3 ms per query, ~60 MB of memory.** The index is one memory-mapped
  file (FSTs, roaring bitmaps): no Redis, no warm-up, only the pages queried
  are read.
- 🌍 **Three countries, one process.** France (BAN), Luxembourg
  (BD-Adresses) and Belgium (BeST Address, in French, Dutch and German),
  selected per request.
- 🔄 **Always up to date.** `update` rebuilds an index only when its source
  changed, or downloads the signed prebuilt one, and the server reloads it
  without a restart.

## At a glance

| Part | What it is |
|---|---|
| [`crates/geocoder-core`](crates/geocoder-core) | The library, [on crates.io](https://crates.io/crates/geocoder-core): index builder, search, reverse, country profiles |
| [`crates/geocoder-api`](crates/geocoder-api) | HTTP API answering like `api-adresse.data.gouv.fr`, one index per country |
| [`chart`](chart) | Helm chart of the API (Deployment, `update` CronJob, Gateway API routing) |
| [`deploy/compose`](deploy/compose) | Docker Compose for a single host, behind Caddy |

## The library

```toml
[dependencies]
geocoder-core = { version = "0.2", default-features = false }
```

```rust
use geocoder_core::{Geocoder, SearchOpts};

let geocoder = Geocoder::open("fr.bin".as_ref())?;
let results = geocoder.search("42 rue de rivoli paris", SearchOpts::default());
```

Building an index, the profiles and the `geocoder-build` /
`geocoder-search` binaries: see the [crate README](crates/geocoder-core/README.md).

### Inside

| Module | Role |
|---|---|
| `search` | addok's `Search` helper, its 13 collectors, result scoring |
| `index` | Memory-mapped index: postings, token pairs, edge ngrams, geohashes, filters, documents |
| `profile` | Everything country specific, behind the `Profile` trait |
| `readers` | Data sources converted to addok's document format |
| `text` | Language-agnostic primitives: ASCII folding, tokens, synonyms, ngrams |

**Profiles.** Every `Profile` method defaults to core addok; a country
overrides only what differs:

- `default`: core addok, no phonetics, no synonyms.
- `fr`: the Géoplateforme configuration: addok-fr phonetics and synonyms,
  addok-france ordinals, housenumbers, cedex/BP cleanup, labels,
  `MIN_SCORE` 0.3.
- `lu`, `be`: the French processing plus 4-digit postcodes (`L-1741`,
  `B-1000`), box numbers (`bte 3`, `bus 2`), the housenumber after the
  street, and an ordinal glued only before a postcode, a street type or
  nothing. `be` indexes every official language of the region, adds Dutch
  abbreviations and labels `Rue de la Loi 16, 1000 Bruxelles` in the
  language closest to the query.

**Sources.** `addok` (the BAN export), `bano`, `lu` (`addresses.csv`) and
`be` (one zip per region). Registers (`lu`, `be`) have one row per address:
they are grouped into street, locality and municipality documents, each
housenumber keeping its own postcode and city when they differ from its
street's, with an importance derived from address counts.

## geocoder-api

One process, one index per country, answering like the BAN API: GeoJSON
`FeatureCollection`s with the `attribution` and `licence` of each country.

```bash
export GEOCODER_COUNTRIES=fr,lu,be GEOCODER_DATA_DIR=data
cargo run --release -p geocoder-api -- update        # downloads, builds what changed
cargo run --release -p geocoder-api -- serve --port 8080

curl 'localhost:8080/search?q=8+bd+du+port+nanterre'
curl 'localhost:8080/search?q=wetstraat+16+brussel&country=be'
curl 'localhost:8080/reverse?lat=49.6116&lon=6.1319&country=lu'
```

| Route | |
|---|---|
| `GET /search` | `q`, `limit`, `autocomplete`, `lat`, `lon`, `type`, `postcode`, `citycode`, `country` |
| `GET /reverse` | `lat`, `lon`, `limit`, `type`, `postcode`, `citycode`, `country` |
| `GET /countries` | Loaded indexes: documents, sources, build date |
| `GET /scalar`, `/openapi.json` | Interactive documentation, OpenAPI 3.1 |
| `GET /health/live`, `/health/ready` | Probes (ready: an index loaded once one is built, not shutting down) |

<details>
<summary><b>Configuration</b></summary>

| Variable | Default | |
|---|---|---|
| `GEOCODER_COUNTRIES` | `fr` | Countries served, `<code>.bin` in the data dir |
| `GEOCODER_DEFAULT_COUNTRY` | the first | Without `country` |
| `GEOCODER_DATA_DIR` | `data` (`/data` in the image) | Indexes, sidecars, downloads |
| `GEOCODER_SOURCE_URLS_<CC>` | built in | Comma separated URLs or paths, e.g. a mirror |
| `GEOCODER_INDEX_BASE_URL` | | Download signed prebuilt indexes from there instead of building them |
| `GEOCODER_INDEX_PUBLIC_KEYS` | built in | Keys trusted to sign them, comma separated (hex) |
| `GEOCODER_RELOAD_INTERVAL_SECONDS` | 60 | Index files checked for a new build |
| `HOST` / `PORT` | `0.0.0.0` / 8080 (3000 in the image) | |
| `SHUTDOWN_DELAY_SECONDS` | 5 | Readiness fails this long after SIGTERM |
| `SENTRY_DSN`, `GEOCODER_ENV`, `RUST_LOG` | | |

`update` skips a country whose sources did not change (`Last-Modified` /
`ETag`, kept in `<code>.bin.source.json`) and writes the new index
atomically.

</details>

### Prebuilt indexes

Building France takes ~5 GB of memory, serving it ~60 MB. The
[`Indexes`](.github/workflows/build-index.yml) workflow builds every day the
countries whose sources changed and publishes them to object storage:
`v<format>/<cc>.json`, a manifest signed with Ed25519 (country, format and
profile versions, size, SHA-256, sources), and the index it names. With
`GEOCODER_INDEX_BASE_URL`, `update` only downloads:

1. the manifest must be signed by a trusted key and match this build
   (country, format, profile), and must not be older than the local index;
2. the index must have exactly the announced size and SHA-256, and open
   (FST checksums included);
3. only then it replaces the current one.

The storage is not trusted: a tampered file is refused, and the index
reader itself never panics on a damaged file (`tests/corrupted.rs`). Builds
are reproducible: the same sources give the same SHA-256.

```bash
geocoder-api keygen 2> public.txt | gh secret set INDEX_SIGNING_KEY   # once
GEOCODER_SIGNING_KEY=… geocoder-api publish --out out                 # what the CI runs
```

## Deployment

The image is [`creatiwity/geocoder-api`](https://hub.docker.com/r/creatiwity/geocoder-api)
(distroless, non-root).

| | Serving | Building an index |
|---|---|---|
| Memory | ~60 MB private, ~200 MB resident with fr+lu+be | ~5 GB (fr), ~4 GB (be) |
| Time | ~3 ms per query | ~8 min (fr), ~2 min (be), seconds (lu) |
| Disk | 3.5 GB of indexes (fr 3.3 GB) | plus the new index while it downloads or builds |

- **Kubernetes**: the [Helm chart](chart) runs the API and an `update`
  CronJob sharing a volume, with optional ListenerSet / HTTPRoute / auth
  (kgateway or APISIX). After the first install, build the indexes once:
  `kubectl create job --from=cronjob/<fullname>-update <fullname>-init`.
- **Telemetry**, like siren's: Sentry (`SENTRY_DSN`) gets the errors, a
  transaction per request with the geocoder-core spans below it, one per
  job (`update`, `publish`) with a span per country, download and build,
  and the Crons check-ins of the `update` CronJob (monitor
  `geocoder-update`). A W3C `traceparent` joins the caller's trace.
  `OTEL_EXPORTER_OTLP_ENDPOINT` exports the same spans and the logs over
  OTLP, each log with its trace (`OTEL_LOGS_EXPORTER=none`: spans only);
  there too a request joins the caller's `traceparent`. Sampling is set on
  each side: `SENTRY_TRACES_SAMPLE_RATE` for Sentry, the standard
  `OTEL_TRACES_SAMPLER` / `OTEL_TRACES_SAMPLER_ARG` for OTLP (e.g.
  `parentbased_traceidratio` and `0.1`, which follows the caller's
  decision; all traces by default). Logs are not sampled.
  `LOG_FORMAT` sets stdout: `text` (default), `json` or `none`.
- **A single host**: [Docker Compose](deploy/compose) with Caddy (HTTPS)
  and a cron line for the updates.

## Reliability

**France** follows the BAN API (see [Validation](#validation)).
**Luxembourg and Belgium** have no reference API:
`examples/roundtrip.rs` samples the register, writes each address several
ways and checks that the first result is that address (within 30 m,
housenumber matched).

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

## Validation

Both levels run against addok itself.

1. **Pipeline, function by function.** `scripts/addok_oracle.py` runs the
   real addok code (tokens, index boosts, labels, scores) without Redis;
   `scripts/diff_pipeline.py` compares it with the Rust implementation
   (`examples/pipeline.rs`) on documents sampled from a BAN export.

   ```bash
   uv venv -p 3.12 .oracle
   VIRTUAL_ENV=.oracle uv pip install addok==1.3.2 addok-fr==1.1.0 addok-france==1.2.0
   cargo build --release --example pipeline
   python3 scripts/diff_pipeline.py --oracle .oracle/bin/python \
       --data data/adresses-addok-france.ndjson.gz --every 97 --max-docs 25000
   ```

2. **End to end.** `scripts/compare_api.py` sends the same queries to the
   BAN API and to the Rust index and reports how often the first result
   matches.

   ```bash
   python3 scripts/compare_api.py data/index-ban.bin \
       --data data/adresses-addok-france.ndjson.gz --sample 200
   ```

Trace the collectors like addok's `--verbose`:
`GEOCODER_DEBUG=1 geocoder-search data/fr.bin rebenac lag`.

## Contributing

`cargo fmt`, `cargo clippy --workspace --all-targets` and `cargo test
--workspace` must pass (the CI runs them). Releases are described in
[`RELEASING.md`](RELEASING.md), upcoming countries in
[`ROADMAP.md`](ROADMAP.md).

## Licence

MIT, see [`LICENSE`](LICENSE). geocoder-core ports addok, addok-fr and
addok-france (MIT, Etalab / DINUM).

The data keep their producers' licences: the BAN (Etalab 2.0),
BD-Adresses (CC0) and BeST Address (FPS BOSA, CC BY 4.0, attribution
required). The API returns the attribution of each country with every
response.
