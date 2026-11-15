//! The indexes served, one per country, reloaded when their file changes.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

use geocoder_core::Geocoder;
use tracing::{error, info};

use crate::config::Config;

/// One country's index.
///
/// `update` replaces the file atomically (rename); the handle notices the
/// new modification time and swaps the index in. Requests keep the index
/// they started with, the previous one is unmapped when the last of them
/// ends.
pub struct GeocoderHandle {
    path: PathBuf,
    state: RwLock<State>,
}

#[derive(Default)]
struct State {
    geocoder: Option<Arc<Geocoder>>,
    modified: Option<SystemTime>,
}

impl GeocoderHandle {
    /// A missing file is not an error: it is picked up once built.
    pub fn open(path: PathBuf) -> Self {
        let handle = Self {
            path,
            state: RwLock::default(),
        };
        handle.reload_if_changed();
        handle
    }

    /// The current index, `None` until one is loaded.
    pub fn get(&self) -> Option<Arc<Geocoder>> {
        self.state.read().expect("lock poisoned").geocoder.clone()
    }

    /// Load the index file if it changed since the last load. Returns
    /// whether a new index is now served.
    pub fn reload_if_changed(&self) -> bool {
        let Ok(modified) = std::fs::metadata(&self.path).and_then(|m| m.modified()) else {
            return false;
        };
        if self.state.read().expect("lock poisoned").modified == Some(modified) {
            return false;
        }
        match Geocoder::open(&self.path) {
            Ok(geocoder) => {
                let meta = geocoder.meta();
                info!(
                    "Index loaded from {} ({} documents, profile {} v{})",
                    self.path.display(),
                    meta.documents,
                    meta.profile,
                    meta.profile_version
                );
                let mut state = self.state.write().expect("lock poisoned");
                state.geocoder = Some(Arc::new(geocoder));
                state.modified = Some(modified);
                true
            }
            Err(e) => {
                // Keep serving the previous index; retried on next change.
                error!("Unable to load index {}: {e}", self.path.display());
                self.state.write().expect("lock poisoned").modified = Some(modified);
                false
            }
        }
    }
}

/// Every configured country's index.
pub struct Indexes {
    handles: BTreeMap<String, Arc<GeocoderHandle>>,
}

impl Indexes {
    pub fn open(config: &Config) -> Arc<Self> {
        let handles = config
            .countries
            .iter()
            .map(|c| {
                (
                    c.code.clone(),
                    Arc::new(GeocoderHandle::open(c.index_path.clone())),
                )
            })
            .collect();
        Arc::new(Self { handles })
    }

    pub fn get(&self, code: &str) -> Option<&Arc<GeocoderHandle>> {
        self.handles.get(code)
    }

    pub fn loaded(&self) -> usize {
        self.handles.values().filter(|h| h.get().is_some()).count()
    }

    /// Check the index files every `every`, in the background.
    pub fn watch(self: &Arc<Self>, every: Duration) {
        let indexes = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(every);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                for handle in indexes.handles.values() {
                    let handle = Arc::clone(handle);
                    // Opening maps the file and copies its FSTs: off the runtime.
                    let _ = tokio::task::spawn_blocking(move || handle.reload_if_changed()).await;
                }
            }
        });
    }
}
