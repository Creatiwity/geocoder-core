//! Which countries are served, from where their data comes, and where their
//! index lives.
//!
//! Every country has built-in defaults (source, profile, attribution), so
//! `GEOCODER_COUNTRIES=fr,lu,be` is enough; `GEOCODER_SOURCE_URLS_<CC>`
//! overrides a country's source (comma separated URLs or local paths).
//!
//! With `GEOCODER_INDEX_BASE_URL`, `update` downloads prebuilt indexes
//! instead (`<base>/v<format>/<cc>.json`, a signed manifest, and the index
//! it names), accepted only when signed by one of the trusted keys. The
//! format version in the path lets builds of several formats coexist.

use std::path::{Path, PathBuf};

use geocoder_core::readers::Source;
use geocoder_core::signed::PublicKey;

/// Keys trusted by default to sign prebuilt indexes (the CI's), hex
/// encoded. `GEOCODER_INDEX_PUBLIC_KEYS` replaces them, e.g. for a rotation.
pub const TRUSTED_KEYS: &[&str] = &[];

#[derive(Debug, Clone)]
pub struct Country {
    /// ISO 3166-1 alpha-2, lowercase: the `country` query parameter.
    pub code: String,
    pub source: Source,
    pub profile: &'static str,
    pub urls: Vec<String>,
    pub index_path: PathBuf,
    /// GeoJSON `attribution` and `licence`, like the BAN API.
    pub attribution: &'static str,
    pub licence: &'static str,
    /// "Rue de la Loi 16" rather than "16 Rue de la Loi" in `name`.
    pub number_after_street: bool,
}

struct Defaults {
    source: Source,
    profile: &'static str,
    urls: &'static [&'static str],
    attribution: &'static str,
    licence: &'static str,
    number_after_street: bool,
}

fn defaults(code: &str) -> Option<Defaults> {
    Some(match code {
        "fr" => Defaults {
            source: Source::Addok,
            profile: "fr",
            urls: &[
                "https://adresse.data.gouv.fr/data/ban/adresses/latest/addok/adresses-addok-france.ndjson.gz",
            ],
            attribution: "BAN",
            licence: "ETALAB-2.0",
            number_after_street: false,
        },
        "lu" => Defaults {
            source: Source::Lu,
            profile: "lu",
            // Stable link to the latest `addresses.csv` of the dataset.
            urls: &["https://data.public.lu/fr/datasets/r/5cadc5b8-6a7d-4283-87bc-f9e58dd771f7"],
            attribution: "BD-Adresses, Administration du cadastre et de la topographie",
            licence: "CC0-1.0",
            number_after_street: false,
        },
        "be" => Defaults {
            source: Source::Be,
            profile: "be",
            urls: &[
                "https://opendata.bosa.be/download/best/openaddress-bevlg.zip",
                "https://opendata.bosa.be/download/best/openaddress-bewal.zip",
                "https://opendata.bosa.be/download/best/openaddress-bebru.zip",
            ],
            attribution: "BeST Address, SPF BOSA",
            licence: "CC-BY-4.0",
            number_after_street: true,
        },
        _ => return None,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("unknown country {0} (known: fr, lu, be)")]
    UnknownCountry(String),
    #[error("no country configured")]
    Empty,
    #[error("default country {0} is not among the configured ones")]
    DefaultNotConfigured(String),
    #[error("invalid public key {0}")]
    PublicKey(String),
    #[error("GEOCODER_INDEX_BASE_URL is set but no public key is trusted")]
    NoTrustedKey,
}

/// Where prebuilt indexes come from, and who may sign them.
#[derive(Debug, Clone)]
pub struct Prebuilt {
    /// `<GEOCODER_INDEX_BASE_URL>/v<index format version>`.
    pub base_url: String,
    pub keys: Vec<PublicKey>,
}

impl Prebuilt {
    /// `keys`: comma separated, hex; `None` for the built-in ones.
    pub fn new(base_url: &str, keys: Option<&str>) -> Result<Option<Self>, ConfigError> {
        let base_url = base_url.trim().trim_end_matches('/');
        if base_url.is_empty() {
            return Ok(None);
        }
        let keys: Vec<&str> = match keys {
            Some(keys) => keys
                .split(',')
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .collect(),
            None => TRUSTED_KEYS.to_vec(),
        };
        let keys = keys
            .into_iter()
            .map(|k| k.parse().map_err(|_| ConfigError::PublicKey(k.to_string())))
            .collect::<Result<Vec<PublicKey>, _>>()?;
        if keys.is_empty() {
            return Err(ConfigError::NoTrustedKey);
        }
        Ok(Some(Self {
            base_url: format!("{base_url}/v{}", geocoder_core::index::format::VERSION),
            keys,
        }))
    }

    pub fn from_env() -> Result<Option<Self>, ConfigError> {
        Self::new(
            &std::env::var("GEOCODER_INDEX_BASE_URL").unwrap_or_default(),
            std::env::var("GEOCODER_INDEX_PUBLIC_KEYS").ok().as_deref(),
        )
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub countries: Vec<Country>,
    pub default_country: String,
}

impl Config {
    /// `countries`: comma separated codes; `source_urls`: the override of a
    /// country, if any.
    pub fn new(
        countries: &str,
        default_country: Option<&str>,
        data_dir: &Path,
        source_urls: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ConfigError> {
        let mut out: Vec<Country> = Vec::new();
        for code in countries.split(',').map(|c| c.trim().to_lowercase()) {
            if code.is_empty() || out.iter().any(|c| c.code == code) {
                continue;
            }
            let d = defaults(&code).ok_or_else(|| ConfigError::UnknownCountry(code.clone()))?;
            let urls = match source_urls(&code) {
                Some(urls) => urls
                    .split(',')
                    .map(str::trim)
                    .filter(|u| !u.is_empty())
                    .map(str::to_string)
                    .collect(),
                None => d.urls.iter().map(|u| u.to_string()).collect(),
            };
            out.push(Country {
                index_path: data_dir.join(format!("{code}.bin")),
                code,
                source: d.source,
                profile: d.profile,
                urls,
                attribution: d.attribution,
                licence: d.licence,
                number_after_street: d.number_after_street,
            });
        }
        let first = out.first().ok_or(ConfigError::Empty)?.code.clone();
        let default_country = match default_country.map(|c| c.trim().to_lowercase()) {
            Some(c) if !c.is_empty() => {
                if !out.iter().any(|country| country.code == c) {
                    return Err(ConfigError::DefaultNotConfigured(c));
                }
                c
            }
            _ => first,
        };
        Ok(Self {
            countries: out,
            default_country,
        })
    }

    /// From the environment (`GEOCODER_SOURCE_URLS_<CC>`).
    pub fn from_env(
        countries: &str,
        default_country: Option<&str>,
        data_dir: &Path,
    ) -> Result<Self, ConfigError> {
        Self::new(countries, default_country, data_dir, |code| {
            std::env::var(format!("GEOCODER_SOURCE_URLS_{}", code.to_uppercase())).ok()
        })
    }

    pub fn country(&self, code: &str) -> Option<&Country> {
        self.countries.iter().find(|c| c.code == code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn countries_with_defaults_and_overrides() {
        let config = Config::new(" LU,be,lu ", None, Path::new("/data"), |code| {
            (code == "be").then(|| "/tmp/a.zip, /tmp/b.zip".to_string())
        })
        .unwrap();
        let codes: Vec<&str> = config.countries.iter().map(|c| c.code.as_str()).collect();
        assert_eq!(codes, ["lu", "be"]);
        assert_eq!(config.default_country, "lu");
        assert_eq!(config.countries[0].index_path, Path::new("/data/lu.bin"));
        assert_eq!(config.countries[1].urls, ["/tmp/a.zip", "/tmp/b.zip"]);
    }

    #[test]
    fn rejects_unknown_countries() {
        assert!(Config::new("fr,xx", None, Path::new("/d"), |_| None).is_err());
        assert!(Config::new("fr", Some("be"), Path::new("/d"), |_| None).is_err());
        assert!(Config::new("", None, Path::new("/d"), |_| None).is_err());
    }

    #[test]
    fn prebuilt() {
        let key = geocoder_core::signed::SigningKey::from_seed([1; 32]).public_key();
        assert!(Prebuilt::new(" ", None).unwrap().is_none());
        let p = Prebuilt::new("https://x/", Some(&format!(" {key} ,")))
            .unwrap()
            .unwrap();
        let format = geocoder_core::index::format::VERSION;
        assert_eq!(p.base_url, format!("https://x/v{format}"));
        assert_eq!(p.keys, [key]);
        assert!(Prebuilt::new("https://x", Some("nope")).is_err());
        assert!(Prebuilt::new("https://x", Some("")).is_err());
    }
}
