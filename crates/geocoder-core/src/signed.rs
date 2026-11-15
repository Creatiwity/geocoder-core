//! Signed manifests of prebuilt indexes.
//!
//! An index built once (in CI) is published with a manifest describing it
//! (country, format and profile versions, size, SHA-256, sources), signed
//! with Ed25519. Whoever downloads it checks the signature against the
//! public keys it trusts, then the file against the manifest's hash, before
//! using it: the storage it comes from does not need to be trusted.
//!
//! The manifest travels inside one JSON envelope, signature included, so a
//! reader never sees a manifest with another one's signature:
//!
//! ```json
//! {"manifest": "<manifest JSON, signed as is>", "key": "<hex public key>", "signature": "<hex>"}
//! ```

use std::io::Read;
use std::path::Path;
use std::str::FromStr;

use ed25519_dalek::Signer;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// What a prebuilt index is. Everything a reader needs to decide whether to
/// download it, and to check what it downloaded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// ISO 3166-1 alpha-2, lowercase.
    pub country: String,
    /// `index::format::VERSION` of the builder.
    pub format_version: u32,
    pub profile: String,
    pub profile_version: u32,
    /// File name of the index, next to the manifest.
    pub file: String,
    pub size: u64,
    /// Lowercase hex.
    pub sha256: String,
    pub documents: usize,
    /// RFC 3339.
    pub built_at: String,
    /// The sources it was built from, as last seen.
    pub sources: Vec<SourceVersion>,
}

/// One source of an index, as last seen.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SourceVersion {
    pub url: String,
    pub last_modified: Option<String>,
    pub etag: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    manifest: String,
    key: String,
    signature: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SignatureError {
    #[error("malformed envelope: {0}")]
    Envelope(#[from] serde_json::Error),
    #[error("signed by an untrusted key {0}")]
    UntrustedKey(String),
    #[error("invalid key or signature encoding")]
    Encoding,
    #[error("bad signature")]
    BadSignature,
}

/// A private key, from its 32-byte seed.
pub struct SigningKey(ed25519_dalek::SigningKey);

/// A public key: 64 hex digits.
#[derive(Clone, PartialEq, Eq)]
pub struct PublicKey(ed25519_dalek::VerifyingKey);

impl SigningKey {
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self(ed25519_dalek::SigningKey::from_bytes(&seed))
    }

    pub fn public_key(&self) -> PublicKey {
        PublicKey(self.0.verifying_key())
    }

    /// The seed, hex encoded: what `FromStr` reads back.
    pub fn to_hex(&self) -> String {
        hex(&self.0.to_bytes())
    }
}

impl FromStr for SigningKey {
    type Err = SignatureError;

    /// 64 hex digits (the seed).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from_seed(
            unhex(s.trim()).ok_or(SignatureError::Encoding)?,
        ))
    }
}

impl FromStr for PublicKey {
    type Err = SignatureError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes: [u8; 32] = unhex(s.trim()).ok_or(SignatureError::Encoding)?;
        ed25519_dalek::VerifyingKey::from_bytes(&bytes)
            .map(Self)
            .map_err(|_| SignatureError::Encoding)
    }
}

impl std::fmt::Display for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&hex(self.0.as_bytes()))
    }
}

impl std::fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PublicKey({self})")
    }
}

/// The envelope of `manifest`, signed with `key`.
pub fn sign(manifest: &Manifest, key: &SigningKey) -> String {
    let manifest = serde_json::to_string(manifest).expect("a manifest always serializes");
    let signature = key.0.sign(manifest.as_bytes());
    serde_json::to_string_pretty(&Envelope {
        key: key.public_key().to_string(),
        signature: hex(&signature.to_bytes()),
        manifest,
    })
    .expect("an envelope always serializes")
}

/// The manifest of an envelope, once its signature is checked against one
/// of the `trusted` keys.
pub fn verify(envelope: &[u8], trusted: &[PublicKey]) -> Result<Manifest, SignatureError> {
    let envelope: Envelope = serde_json::from_slice(envelope)?;
    let key: PublicKey = envelope.key.parse()?;
    if !trusted.contains(&key) {
        return Err(SignatureError::UntrustedKey(envelope.key));
    }
    let signature: [u8; 64] = unhex(&envelope.signature).ok_or(SignatureError::Encoding)?;
    // Strict: rejects the malleable and weak-key signatures plain `verify`
    // tolerates.
    key.0
        .verify_strict(
            envelope.manifest.as_bytes(),
            &ed25519_dalek::Signature::from_bytes(&signature),
        )
        .map_err(|_| SignatureError::BadSignature)?;
    Ok(serde_json::from_str(&envelope.manifest)?)
}

/// Incremental SHA-256, e.g. of a download as it streams.
#[derive(Default)]
pub struct Hasher(Sha256);

impl Hasher {
    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    /// Lowercase hex.
    pub fn finish(self) -> String {
        hex(&self.0.finalize())
    }
}

/// SHA-256 (lowercase hex) and size of a file.
pub fn sha256_file(path: &Path) -> std::io::Result<(String, u64)> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Hasher::default();
    let mut buffer = vec![0u8; 1 << 20];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        size += n as u64;
    }
    Ok((hasher.finish(), size))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != 2 * N || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest {
            country: "lu".into(),
            format_version: 2,
            profile: "lu".into(),
            profile_version: 1,
            file: "lu-0123456789abcdef.bin".into(),
            size: 42,
            sha256: "00".repeat(32),
            documents: 7,
            built_at: "2026-11-15T22:00:00Z".into(),
            sources: vec![SourceVersion {
                url: "https://example.org/a.csv".into(),
                last_modified: Some("Sat, 14 Nov 2026 10:00:00 GMT".into()),
                etag: None,
            }],
        }
    }

    #[test]
    fn round_trip() {
        let key = SigningKey::from_seed([7; 32]);
        let envelope = sign(&manifest(), &key);
        assert_eq!(
            verify(envelope.as_bytes(), &[key.public_key()]).unwrap(),
            manifest()
        );
        // The keys read back from their hex form.
        let again: SigningKey = key.to_hex().parse().unwrap();
        assert_eq!(again.public_key(), key.public_key());
        assert_eq!(
            key.public_key().to_string().parse::<PublicKey>().unwrap(),
            key.public_key()
        );
    }

    #[test]
    fn rejects_other_keys_and_tampering() {
        let key = SigningKey::from_seed([7; 32]);
        let other = SigningKey::from_seed([8; 32]);
        let envelope = sign(&manifest(), &key);
        assert!(matches!(
            verify(envelope.as_bytes(), &[other.public_key()]),
            Err(SignatureError::UntrustedKey(_))
        ));

        // The manifest changed after signing (another hash).
        let mut e: Envelope = serde_json::from_str(&envelope).unwrap();
        e.manifest = e.manifest.replace(&"00".repeat(32), &"11".repeat(32));
        let tampered = serde_json::to_vec(&e).unwrap();
        assert!(matches!(
            verify(&tampered, &[key.public_key()]),
            Err(SignatureError::BadSignature)
        ));

        // Signed by an untrusted key that claims to be the trusted one.
        let mut e: Envelope = serde_json::from_str(&sign(&manifest(), &other)).unwrap();
        e.key = key.public_key().to_string();
        let forged = serde_json::to_vec(&e).unwrap();
        assert!(matches!(
            verify(&forged, &[key.public_key()]),
            Err(SignatureError::BadSignature)
        ));
    }

    #[test]
    fn hashes() {
        let mut h = Hasher::default();
        h.update(b"abc");
        assert_eq!(
            h.finish(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
