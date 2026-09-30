# geocoder-core

Address geocoding in Rust, ported from [addok](https://github.com/addok/addok)
to be embedded in other services (e.g. siren). The reference behaviour is
the BAN API as served by the Géoplateforme (IGN): addok 1.3 + addok-fr
1.1.0 + addok-france 1.2.0 with its address index configuration (see
`SCORING.md`).

## Layout

| Part | Role |
|---|---|
| `src/search` | Generic engine: addok's `Search` helper, its 13 collectors, result scoring |
| `src/index` | Memory-mapped index: postings, token pairs, edge ngrams, geohashes, filters, documents |
| `src/profile` | Country specific processing behind the `Profile` trait |
| `src/readers` | Data sources converted to addok's document format |
| `src/text` | Language-agnostic text primitives (ASCII folding, tokenizing, synonyms, ngrams) |

### Profiles

Everything that depends on how addresses are written goes through a
`Profile`: query cleanup, token processing (housenumbers, synonyms,
phonetics), labels, indexed fields and filters. Every method defaults to
core addok's behaviour; a country overrides only what it needs.

- `default`: core addok (no phonetics, no synonyms).
- `fr`: the Géoplateforme configuration (addok-fr phonetics and synonyms,
  addok-france ordinals, housenumber detection, cedex/BP cleanup, labels,
  `MIN_SCORE` 0.3).

The profile name and version are stored in the index; opening an index
built with another version fails, since its tokens would no longer match.

### Sources

- `addok` (default): addok's own format, e.g. the BAN export
  `https://adresse.data.gouv.fr/data/ban/adresses/latest/addok/adresses-addok-france.ndjson.gz`.
- `bano`: BANO files, normalized to the BAN types (`municipality`,
  `street`, `locality`) and `context`.

## Usage

```bash
cargo build --release
./target/release/geocoder-build --source addok --profile fr \
    data/adresses-addok-france.ndjson.gz data/index-ban.bin
./target/release/geocoder-search data/index-ban.bin 42 rue de rivoli paris
echo "bd haussmann paris" | ./target/release/geocoder-search --batch data/index-ban.bin

# Trace the collectors, like addok's --verbose
GEOCODER_DEBUG=1 ./target/release/geocoder-search data/index-ban.bin rebenac lag
```

### As a library

```toml
geocoder-core = { git = "https://github.com/Creatiwity/geocoder-core", tag = "v0.1.0", default-features = false }
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
   (`examples/pipeline.rs`) on documents sampled from a BAN export and
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
