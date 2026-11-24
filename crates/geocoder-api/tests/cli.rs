//! The commands whose stdout scripts read (the `Indexes` workflow) print
//! nothing else there: logs go to stderr.

use std::process::Command;

fn geocoder_api(args: &[&str], env: &[(&str, &str)]) -> (String, String, bool) {
    let output = Command::new(env!("CARGO_BIN_EXE_geocoder-api"))
        .args(args)
        .env_remove("SENTRY_DSN")
        .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
        .env("RUST_LOG", "info")
        .envs(env.iter().copied())
        .output()
        .unwrap();
    (
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
        output.status.success(),
    )
}

fn is_key(line: &str) -> bool {
    line.len() == 64 && line.bytes().all(|b| b.is_ascii_hexdigit())
}

#[test]
fn keygen_prints_only_the_key() {
    let (stdout, stderr, ok) = geocoder_api(&["keygen"], &[]);
    assert!(ok, "{stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "{stdout:?}");
    assert!(is_key(lines[0]), "{stdout:?}");
    // The logs went to stderr.
    assert!(stderr.contains("geocoder-api"), "{stderr:?}");

    let (public, stderr, ok) = geocoder_api(&["public-key"], &[("GEOCODER_SIGNING_KEY", lines[0])]);
    assert!(ok, "{stderr}");
    let lines: Vec<&str> = public.lines().collect();
    assert_eq!(lines.len(), 1, "{public:?}");
    assert!(is_key(lines[0]), "{public:?}");
}
