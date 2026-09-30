//! Debug helper: `inspect <index> <token> [n]` prints the first documents of
//! a (processed) token, in index order, with their boost.

use geocoder_core::Geocoder;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let geocoder = Geocoder::open(std::path::Path::new(&args[1])).unwrap();
    let index = geocoder.index();
    let n: usize = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(20);
    let Some(id) = index.token_id(&args[2]) else {
        println!("token not found");
        return;
    };
    println!(
        "token {} id {id} frequency {}",
        args[2],
        index.frequency(id)
    );
    let postings = index.postings(id);
    for doc in index.top(id, n) {
        let d = index.document(doc).unwrap();
        println!(
            "  doc {doc:>8} boost {:.6}  {} | {} | {}",
            postings.find(doc).unwrap_or_default(),
            d.get("type").and_then(|v| v.as_str()).unwrap_or_default(),
            d.get("name").map(|v| v.to_string()).unwrap_or_default(),
            d.get("importance")
                .map(|v| v.to_string())
                .unwrap_or_default(),
        );
    }
}
