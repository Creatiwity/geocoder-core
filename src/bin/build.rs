use std::io::{BufReader, Read};
use std::path::PathBuf;

use flate2::read::MultiGzDecoder;
use geocoder_core::index::import;
use geocoder_core::readers::Source;
use geocoder_core::{GeocoderError, profile};

const USAGE: &str = "usage: geocoder-build [--source addok|bano] [--profile fr|default] <url|path[.gz]> <output.bin>

  --source   input format: addok (BAN exports, default) or bano
  --profile  text processing profile stored in the index (default: fr)";

fn main() -> Result<(), GeocoderError> {
    let mut args = std::env::args().skip(1);
    let mut source: Option<Source> = None;
    let mut profile_name = "fr".to_string();
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
            "--profile" => profile_name = args.next().unwrap_or_default(),
            "-h" | "--help" => exit(""),
            _ => positional.push(arg),
        }
    }
    let [input, output] = positional.as_slice() else {
        exit("")
    };
    let source = source.unwrap_or(if input.contains("bano") {
        Source::Bano
    } else {
        Source::Addok
    });
    let profile = profile::by_name(&profile_name).unwrap_or_else(|| {
        exit(&format!(
            "unknown profile {profile_name}, known: {:?}",
            profile::names()
        ))
    });
    let output: PathBuf = output.into();

    let raw: Box<dyn Read> = if input.starts_with("http://") || input.starts_with("https://") {
        eprintln!("streaming {input}");
        let response = ureq::get(input)
            .call()
            .map_err(|e| GeocoderError::Io(std::io::Error::other(e.to_string())))?;
        Box::new(response.into_reader())
    } else {
        Box::new(std::fs::File::open(input)?)
    };
    let reader: Box<dyn Read> = if input.ends_with(".gz") {
        Box::new(MultiGzDecoder::new(raw))
    } else {
        raw
    };
    let reader = BufReader::with_capacity(1 << 20, reader);

    let stats = import(reader, source, profile, &output, |p| {
        eprintln!(
            "{} lines, {} indexed, {:?}",
            p.lines, p.documents, p.elapsed
        );
    })?;
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

fn exit(message: &str) -> ! {
    if !message.is_empty() {
        eprintln!("{message}");
    }
    eprintln!("{USAGE}");
    std::process::exit(1)
}
