use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use geocoder_core::{Document, GeocoderError, IndexBuilder};

fn main() -> Result<(), GeocoderError> {
    let mut args = std::env::args().skip(1);
    let input_path: PathBuf = args.next().expect("usage: geocoder-build <input.ndjson> <output.bin>").into();
    let output_path: PathBuf = args.next().expect("usage: geocoder-build <input.ndjson> <output.bin>").into();

    let file = std::fs::File::open(&input_path)?;
    let reader = BufReader::new(file);

    let mut builder = IndexBuilder::new();
    let mut count = 0usize;

    for (line_no, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let doc: Document = serde_json::from_str(&line).map_err(|e| {
            eprintln!("line {}: {}", line_no + 1, e);
            GeocoderError::Json(e)
        })?;
        builder.add_document(doc)?;
        count += 1;
        if count % 100_000 == 0 {
            eprintln!("indexed {} documents...", count);
        }
    }

    eprintln!("building index ({} documents)...", count);
    builder.build(&output_path)?;
    eprintln!("done → {}", output_path.display());
    Ok(())
}
