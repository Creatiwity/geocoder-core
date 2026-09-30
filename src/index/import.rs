//! Build an index file from a data source, atomically.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::IndexBuilder;
use crate::error::GeocoderError;
use crate::profile::Profile;
use crate::readers::Source;

/// Progress, reported every `PROGRESS_EVERY` lines.
#[derive(Debug, Clone, Copy)]
pub struct ImportProgress {
    pub lines: usize,
    pub documents: usize,
    pub elapsed: Duration,
}

#[derive(Debug, Clone, Copy)]
pub struct ImportStats {
    pub documents: usize,
    /// Rejected documents (missing required field or coordinates).
    pub skipped: usize,
    /// Lines that could not be parsed.
    pub invalid: usize,
    pub elapsed: Duration,
}

pub const PROGRESS_EVERY: usize = 200_000;

/// Index every line of `reader` and write the index to `output`.
///
/// The index is written to a temporary file next to `output`, then renamed:
/// a process opening `output` meanwhile sees the previous index, never a
/// partial one. Invalid lines are counted and skipped.
pub fn import(
    reader: impl BufRead,
    source: Source,
    profile: Arc<dyn Profile>,
    output: &Path,
    mut progress: impl FnMut(ImportProgress),
) -> Result<ImportStats, GeocoderError> {
    let started = Instant::now();
    let mut builder = IndexBuilder::new(profile);
    let mut invalid = 0usize;
    for (n, line) in reader.lines().enumerate() {
        let line = line?;
        match source.parse(&line) {
            Ok(Some(doc)) => {
                builder.add_document(doc)?;
            }
            Ok(None) => {}
            Err(_) => invalid += 1,
        }
        if (n + 1) % PROGRESS_EVERY == 0 {
            progress(ImportProgress {
                lines: n + 1,
                documents: builder.documents(),
                elapsed: started.elapsed(),
            });
        }
    }
    let (documents, skipped) = (builder.documents(), builder.skipped());
    if documents == 0 {
        return Err(GeocoderError::InvalidIndex("no document to index"));
    }

    let tmp = temporary_path(output);
    if let Err(e) = builder.build(&tmp) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, output)?;
    Ok(ImportStats {
        documents,
        skipped,
        invalid,
        elapsed: started.elapsed(),
    })
}

fn temporary_path(output: &Path) -> PathBuf {
    let mut name = output
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".tmp");
    output.with_file_name(name)
}
