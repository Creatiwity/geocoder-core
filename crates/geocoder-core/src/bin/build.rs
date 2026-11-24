use std::path::{Path, PathBuf};

use geocoder_core::index::import_files;
use geocoder_core::readers::Source;
use geocoder_core::{GeocoderError, profile};

const USAGE: &str = "usage: geocoder-build [--source addok|bano|lu|be] [--profile fr|lu|be|default] <url|path>... <output.bin>

  --source   input format: addok (BAN exports, default), bano, lu
             (BD-Adresses CSV) or be (BeST CSV, one zip per region)
  --profile  text processing profile stored in the index (default: the
             source's country, fr for addok and bano)

Inputs may be plain, .gz or .zip files; URLs are downloaded next to the
output first.";

fn main() -> Result<(), GeocoderError> {
    let mut args = std::env::args().skip(1);
    let mut source: Option<Source> = None;
    let mut profile_name: Option<String> = None;
    let mut positional: Vec<String> = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--source" | "--format" => {
                let name = args.next().unwrap_or_default();
                source = Some(
                    Source::from_name(&name)
                        .unwrap_or_else(|| exit(&format!("unknown source {name}"))),
                );
            }
            "--profile" => profile_name = args.next(),
            "-h" | "--help" => exit(""),
            _ => positional.push(arg),
        }
    }
    let Some((output, inputs)) = positional.split_last() else {
        exit("")
    };
    if inputs.is_empty() {
        exit("")
    }
    let source = source.unwrap_or(if inputs[0].contains("bano") {
        Source::Bano
    } else {
        Source::Addok
    });
    let profile_name = profile_name.unwrap_or_else(|| default_profile(source).to_string());
    let profile = profile::by_name(&profile_name).unwrap_or_else(|| {
        exit(&format!(
            "unknown profile {profile_name}, known: {:?}",
            profile::names()
        ))
    });
    let output = PathBuf::from(output);

    let mut downloads = Vec::new();
    let mut files = Vec::new();
    for (i, input) in inputs.iter().enumerate() {
        if input.starts_with("http://") || input.starts_with("https://") {
            let target = download(input, &output, i)?;
            downloads.push(target.clone());
            files.push(target);
        } else {
            files.push(PathBuf::from(input));
        }
    }

    let result = import_files(&files, source, profile, &output, None, |p| {
        eprintln!(
            "{} lines, {} indexed, {:?}",
            p.lines, p.documents, p.elapsed
        );
    });
    for file in downloads {
        let _ = std::fs::remove_file(file);
    }
    let stats = result?;
    eprintln!(
        "{} documents indexed, {} skipped, {} invalid lines, written to {} in {:?}",
        stats.documents,
        stats.skipped,
        stats.invalid,
        output.display(),
        stats.elapsed
    );
    Ok(())
}

fn default_profile(source: Source) -> &'static str {
    match source {
        Source::Addok | Source::Bano => "fr",
        Source::Lu => "lu",
        Source::Be => "be",
    }
}

/// Download next to the output, keeping the extension (.gz, .zip).
fn download(url: &str, output: &Path, n: usize) -> Result<PathBuf, GeocoderError> {
    let extension = url
        .rsplit('/')
        .next()
        .and_then(|name| name.split('?').next())
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, ext)| ext.to_string())
        .unwrap_or_default();
    let mut name = output.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".download{n}.{extension}"));
    let target = output.with_file_name(name);
    eprintln!("downloading {url}");
    let response = ureq::get(url)
        .call()
        .map_err(|e| GeocoderError::Io(std::io::Error::other(e.to_string())))?;
    let mut file = std::fs::File::create(&target)?;
    // Unlike read_to_*, the reader has no size limit (the BAN export is GBs).
    std::io::copy(&mut response.into_body().into_reader(), &mut file)?;
    Ok(target)
}

fn exit(message: &str) -> ! {
    if !message.is_empty() {
        eprintln!("{message}");
    }
    eprintln!("{USAGE}");
    std::process::exit(1)
}
