//! Build an index file from a data source, atomically.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::IndexBuilder;
use crate::doc::Document;
use crate::error::GeocoderError;
use crate::profile::Profile;
use crate::readers::{Source, be, for_each_input, grouped::Grouper, lu};

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

/// Index every document of `reader` and write the index to `output`.
///
/// The index is written to a temporary file next to `output`, then renamed:
/// a process opening `output` meanwhile sees the previous index, never a
/// partial one. Invalid lines are counted and skipped.
pub fn import(
    reader: impl BufRead,
    source: Source,
    profile: Arc<dyn Profile>,
    output: &Path,
    progress: impl FnMut(ImportProgress),
) -> Result<ImportStats, GeocoderError> {
    let span = import_span(source, output);
    let _entered = span.enter();
    let mut importer = Importer::new(profile, progress);
    let mut reader = reader;
    importer.read(&mut reader, source)?;
    record(&span, importer.finish(output))
}

/// [`import`] over several files (e.g. the three regions of Belgium), each
/// a plain, `.gz` or `.zip` file.
pub fn import_files(
    inputs: &[PathBuf],
    source: Source,
    profile: Arc<dyn Profile>,
    output: &Path,
    progress: impl FnMut(ImportProgress),
) -> Result<ImportStats, GeocoderError> {
    let span = import_span(source, output);
    let _entered = span.enter();
    let mut importer = Importer::new(profile, progress);
    for input in inputs {
        for_each_input(input, |reader| importer.read(reader, source))?;
    }
    record(&span, importer.finish(output))
}

/// `geocoder.import` (INFO), around reading the sources and writing the
/// index (`geocoder.write`).
fn import_span(source: Source, output: &Path) -> tracing::Span {
    tracing::info_span!(
        "geocoder.import",
        sentry.op = "geocoder.import",
        source = source.name(),
        output = %output.display(),
        documents = tracing::field::Empty,
        skipped = tracing::field::Empty,
        invalid = tracing::field::Empty,
    )
}

fn record(
    span: &tracing::Span,
    stats: Result<ImportStats, GeocoderError>,
) -> Result<ImportStats, GeocoderError> {
    if let Ok(stats) = &stats {
        span.record("documents", stats.documents);
        span.record("skipped", stats.skipped);
        span.record("invalid", stats.invalid);
    }
    stats
}

struct Importer<P> {
    builder: IndexBuilder,
    progress: P,
    started: Instant,
    lines: usize,
    invalid: usize,
    /// A register's streets, turned into documents once every input is
    /// read (Belgium comes in one file per region).
    grouper: Option<Grouper>,
}

impl<P: FnMut(ImportProgress)> Importer<P> {
    fn new(profile: Arc<dyn Profile>, progress: P) -> Self {
        Self {
            builder: IndexBuilder::new(profile),
            progress,
            started: Instant::now(),
            lines: 0,
            invalid: 0,
            grouper: None,
        }
    }

    fn read(&mut self, reader: &mut dyn Read, source: Source) -> Result<(), GeocoderError> {
        match source {
            Source::Addok | Source::Bano => {
                for line in BufReader::new(reader).lines() {
                    let line = line?;
                    match source.parse(&line) {
                        Ok(Some(doc)) => {
                            self.builder.add_document(doc)?;
                        }
                        Ok(None) => {}
                        Err(_) => self.invalid += 1,
                    }
                    self.tick();
                }
            }
            Source::Lu | Source::Be => {
                let grouper = self
                    .grouper
                    .get_or_insert_with(|| Grouper::new(source.name()));
                let mut lines = 0usize;
                let invalid = if source == Source::Lu {
                    lu::for_each_row(reader, |row| {
                        grouper.add(row);
                        lines += 1;
                    })?
                } else {
                    be::for_each_row(reader, |row| {
                        grouper.add(row);
                        lines += 1;
                    })?
                };
                self.invalid += invalid;
                self.lines += lines;
            }
        }
        Ok(())
    }

    fn add_all(&mut self, docs: Vec<Document>) -> Result<(), GeocoderError> {
        for doc in docs {
            self.builder.add_document(doc)?;
            self.tick();
        }
        Ok(())
    }

    fn tick(&mut self) {
        self.lines += 1;
        if self.lines.is_multiple_of(PROGRESS_EVERY) {
            (self.progress)(ImportProgress {
                lines: self.lines,
                documents: self.builder.documents(),
                elapsed: self.started.elapsed(),
            });
        }
    }

    fn finish(mut self, output: &Path) -> Result<ImportStats, GeocoderError> {
        if let Some(grouper) = self.grouper.take() {
            self.add_all(grouper.finish())?;
        }
        let (documents, skipped) = (self.builder.documents(), self.builder.skipped());
        if documents == 0 {
            return Err(GeocoderError::InvalidIndex("no document to index"));
        }

        let tmp = temporary_path(output);
        let built = tracing::info_span!("geocoder.write", sentry.op = "geocoder.write", documents)
            .in_scope(|| self.builder.build(&tmp));
        if let Err(e) = built {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        std::fs::rename(&tmp, output)?;
        Ok(ImportStats {
            documents,
            skipped,
            invalid: self.invalid,
            elapsed: self.started.elapsed(),
        })
    }
}

fn temporary_path(output: &Path) -> PathBuf {
    let mut name = output
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".tmp");
    output.with_file_name(name)
}
