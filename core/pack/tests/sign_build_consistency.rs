//! Integration test: `actr pkg sign` and `actr pkg build` signature consistency.
//!
//! Verifies that the offline `sign` workflow produces output that is
//! byte-level identical and verification-compatible with the `build` workflow.
//!
//! Scenarios:
//! 1. sign and build produce identical manifest TOML (same key, same inputs)
//! 2. sign's manifest + sig can be assembled into a verifiable .actr package
//! 3. sign and build both pass actr_pack::verify with the same public key
//! 4. sign and build with proto files produce identical manifest TOML
//! 5. signing_key_id is present and consistent in both workflows

use actr_pack::manifest::{BinaryEntry, ManifestMetadata, PackageManifest, ProtoFileEntry};
use actr_pack::{PackOptions, pack, verify};
use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use std::io::{Cursor, Write};
use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

/// Simulate the `actr pkg sign` workflow:
/// Given manifest.toml fields + binary + protos + signing key,
/// build a PackageManifest, serialize via to_toml(), sign the manifest bytes.
/// Returns (manifest_toml_string, 64-byte raw signature, manifest struct).
fn simulate_sign(
    manufacturer: &str,
    name: &str,
    version: &str,
    target: &str,
    binary_bytes: &[u8],
    proto_files: &[(String, Vec<u8>)],
    signing_key: &SigningKey,
) -> (String, [u8; 64], PackageManifest) {
    let verifying_key = signing_key.verifying_key();
    let key_id = actr_pack::compute_key_id(&verifying_key.to_bytes());

    // Compute binary hash (same as sign command: sha2::Sha256 + hex::encode)
    let binary_hash = hex::encode(Sha256::digest(binary_bytes));

    // Compute proto file entries with hashes (same as sign command)
    let proto_entries: Vec<ProtoFileEntry> = proto_files
        .iter()
        .map(|(fname, content)| ProtoFileEntry {
            name: fname.clone(),
            path: format!("proto/{}", fname),
            hash: hex::encode(Sha256::digest(content)),
        })
        .collect();

    // Build PackageManifest (same structure as sign command)
    let manifest = PackageManifest {
        manufacturer: manufacturer.to_string(),
        name: name.to_string(),
        version: version.to_string(),
        binary: BinaryEntry {
            path: "bin/actor.wasm".to_string(),
            target: target.to_string(),
            hash: binary_hash,
            size: Some(binary_bytes.len() as u64),
        },
        signature_algorithm: "ed25519".to_string(),
        signing_key_id: Some(key_id),
        resources: vec![],
        proto_files: proto_entries,
        lock_file: None,
        metadata: ManifestMetadata::default(),
    };

    // Serialize via to_toml() (same as sign command)
    let manifest_toml = manifest.to_toml().unwrap();

    // Sign (raw 64-byte Ed25519, same as sign command)
    let signature = signing_key.sign(manifest_toml.as_bytes());
    let sig_bytes = signature.to_bytes();

    (manifest_toml, sig_bytes, manifest)
}

/// Simulate the `actr pkg build` workflow using actr_pack::pack.
/// Returns the .actr package bytes.
fn simulate_build(
    manufacturer: &str,
    name: &str,
    version: &str,
    target: &str,
    binary_bytes: &[u8],
    proto_files: Vec<(String, Vec<u8>)>,
    signing_key: &SigningKey,
) -> Vec<u8> {
    let verifying_key = signing_key.verifying_key();
    let key_id = actr_pack::compute_key_id(&verifying_key.to_bytes());

    let manifest = PackageManifest {
        manufacturer: manufacturer.to_string(),
        name: name.to_string(),
        version: version.to_string(),
        binary: BinaryEntry {
            path: "bin/actor.wasm".to_string(),
            target: target.to_string(),
            hash: String::new(), // pack() computes this
            size: None,          // pack() computes this
        },
        signature_algorithm: "ed25519".to_string(),
        signing_key_id: Some(key_id),
        resources: vec![],
        proto_files: vec![], // pack() computes this from proto_files input
        lock_file: None,
        metadata: ManifestMetadata::default(),
    };

    let opts = PackOptions {
        manifest,
        binary_bytes: binary_bytes.to_vec(),
        resources: vec![],
        proto_files,
        signing_key: signing_key.clone(),
        lock_file: None,
    };

    pack(&opts).unwrap()
}

/// Assemble a .actr package from sign's output (manifest TOML + sig + binary + protos).
/// This simulates what a user would need to do to create a verifiable .actr from sign output.
fn assemble_actr_from_sign_output(
    manifest_toml: &str,
    sig_bytes: &[u8; 64],
    binary_bytes: &[u8],
    proto_files: &[(String, Vec<u8>)],
) -> Vec<u8> {
    let buf = Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(buf);
    let store_opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

    // manifest.toml
    zip.start_file("manifest.toml", store_opts).unwrap();
    zip.write_all(manifest_toml.as_bytes()).unwrap();

    // manifest.sig
    zip.start_file("manifest.sig", store_opts).unwrap();
    zip.write_all(sig_bytes).unwrap();

    // binary
    zip.start_file("bin/actor.wasm", store_opts).unwrap();
    zip.write_all(binary_bytes).unwrap();

    // proto files
    for (name, content) in proto_files {
        let zip_path = format!("proto/{}", name);
        zip.start_file(&zip_path, store_opts).unwrap();
        zip.write_all(content).unwrap();
    }

    let cursor = zip.finish().unwrap();
    cursor.into_inner()
}

// ─── Test cases ──────────────────────────────────────────────────────────────

/// Test 1: sign and build produce identical manifest TOML content
#[test]
fn sign_and_build_produce_identical_manifest() {
    let key = SigningKey::generate(&mut OsRng);
    let binary = b"fake wasm binary content";

    // sign workflow
    let (sign_manifest_toml, _, _) = simulate_sign(
        "acme",
        "EchoService",
        "1.0.0",
        "wasm32-wasip1",
        binary,
        &[],
        &key,
    );

    // build workflow — extract manifest from .actr package
    let build_pkg = simulate_build(
        "acme",
        "EchoService",
        "1.0.0",
        "wasm32-wasip1",
        binary,
        vec![],
        &key,
    );
    let build_manifest_raw = actr_pack::read_manifest_raw(&build_pkg).unwrap();

    assert_eq!(
        sign_manifest_toml, build_manifest_raw,
        "sign and build should produce byte-identical manifest TOML"
    );
}

/// Test 2: sign output can be assembled into a package that passes verify
#[test]
fn sign_output_passes_verification() {
    let key = SigningKey::generate(&mut OsRng);
    let binary = b"hello wasm";

    let (manifest_toml, sig_bytes, _) = simulate_sign(
        "acme",
        "EchoService",
        "1.0.0",
        "wasm32-wasip1",
        binary,
        &[],
        &key,
    );

    // Assemble .actr from sign output
    let actr_pkg = assemble_actr_from_sign_output(&manifest_toml, &sig_bytes, binary, &[]);

    // Verify with actr_pack::verify
    let result = verify(&actr_pkg, &key.verifying_key());
    assert!(
        result.is_ok(),
        "sign output assembled into .actr should pass verify: {:?}",
        result.err()
    );

    let verified = result.unwrap();
    assert_eq!(verified.manifest.manufacturer, "acme");
    assert_eq!(verified.manifest.name, "EchoService");
    assert_eq!(verified.manifest.version, "1.0.0");
}

/// Test 3: both sign and build packages pass verify with the same key
#[test]
fn both_sign_and_build_pass_verify() {
    let key = SigningKey::generate(&mut OsRng);
    let binary = b"wasm binary data";

    // Build package
    let build_pkg = simulate_build(
        "acme",
        "Sensor",
        "2.0.0",
        "wasm32-wasip1",
        binary,
        vec![],
        &key,
    );
    let build_result = verify(&build_pkg, &key.verifying_key());
    assert!(build_result.is_ok(), "build package should verify");

    // Sign + assemble package
    let (manifest_toml, sig_bytes, _) = simulate_sign(
        "acme",
        "Sensor",
        "2.0.0",
        "wasm32-wasip1",
        binary,
        &[],
        &key,
    );
    let sign_pkg = assemble_actr_from_sign_output(&manifest_toml, &sig_bytes, binary, &[]);
    let sign_result = verify(&sign_pkg, &key.verifying_key());
    assert!(sign_result.is_ok(), "sign package should verify");

    // Both should produce identical manifest content
    let build_verified = build_result.unwrap();
    let sign_verified = sign_result.unwrap();
    assert_eq!(build_verified.manifest_raw, sign_verified.manifest_raw);
}

/// Test 4: sign and build with proto files produce identical manifest
#[test]
fn sign_and_build_with_protos_produce_identical_manifest() {
    let key = SigningKey::generate(&mut OsRng);
    let binary = b"wasm binary";
    let protos = vec![
        (
            "echo.proto".to_string(),
            b"syntax = \"proto3\";\nservice Echo {}".to_vec(),
        ),
        (
            "common.proto".to_string(),
            b"syntax = \"proto3\";\nmessage Empty {}".to_vec(),
        ),
    ];

    // sign workflow
    let (sign_manifest_toml, _, _) = simulate_sign(
        "acme",
        "EchoService",
        "1.0.0",
        "wasm32-wasip1",
        binary,
        &protos,
        &key,
    );

    // build workflow
    let build_pkg = simulate_build(
        "acme",
        "EchoService",
        "1.0.0",
        "wasm32-wasip1",
        binary,
        protos.clone(),
        &key,
    );
    let build_manifest_raw = actr_pack::read_manifest_raw(&build_pkg).unwrap();

    assert_eq!(
        sign_manifest_toml, build_manifest_raw,
        "sign and build with protos should produce byte-identical manifest"
    );
}

/// Test 5: sign with protos assembled into .actr passes verify
#[test]
fn sign_with_protos_passes_verification() {
    let key = SigningKey::generate(&mut OsRng);
    let binary = b"wasm binary";
    let protos = vec![(
        "echo.proto".to_string(),
        b"syntax = \"proto3\";\nservice Echo {}".to_vec(),
    )];

    let (manifest_toml, sig_bytes, _) = simulate_sign(
        "acme",
        "EchoService",
        "1.0.0",
        "wasm32-wasip1",
        binary,
        &protos,
        &key,
    );

    let actr_pkg = assemble_actr_from_sign_output(&manifest_toml, &sig_bytes, binary, &protos);
    let result = verify(&actr_pkg, &key.verifying_key());
    assert!(
        result.is_ok(),
        "sign+protos assembled .actr should verify: {:?}",
        result.err()
    );

    let verified = result.unwrap();
    assert_eq!(verified.manifest.proto_files.len(), 1);
    assert_eq!(verified.manifest.proto_files[0].name, "echo.proto");
}

/// Test 6: signing_key_id is present and consistent in both workflows
#[test]
fn signing_key_id_consistent_between_sign_and_build() {
    let key = SigningKey::generate(&mut OsRng);
    let expected_key_id = actr_pack::compute_key_id(&key.verifying_key().to_bytes());
    let binary = b"wasm";

    // sign workflow
    let (_, _, sign_manifest) =
        simulate_sign("acme", "App", "1.0.0", "wasm32-wasip1", binary, &[], &key);
    assert_eq!(
        sign_manifest.signing_key_id.as_deref(),
        Some(expected_key_id.as_str()),
        "sign should have correct signing_key_id"
    );

    // build workflow
    let build_pkg = simulate_build(
        "acme",
        "App",
        "1.0.0",
        "wasm32-wasip1",
        binary,
        vec![],
        &key,
    );
    let build_manifest = actr_pack::read_manifest(&build_pkg).unwrap();
    assert_eq!(
        build_manifest.signing_key_id.as_deref(),
        Some(expected_key_id.as_str()),
        "build should have correct signing_key_id"
    );

    // Same key_id
    assert_eq!(
        sign_manifest.signing_key_id, build_manifest.signing_key_id,
        "sign and build should produce identical signing_key_id"
    );
}

/// Test 7: sign output with wrong key fails verification
#[test]
fn sign_output_with_wrong_key_fails_verification() {
    let key_a = SigningKey::generate(&mut OsRng);
    let key_b = SigningKey::generate(&mut OsRng);
    let binary = b"wasm";

    // Sign with key_a
    let (manifest_toml, sig_bytes, _) =
        simulate_sign("acme", "App", "1.0.0", "wasm32-wasip1", binary, &[], &key_a);

    // Assemble and verify with key_b → should fail
    let actr_pkg = assemble_actr_from_sign_output(&manifest_toml, &sig_bytes, binary, &[]);
    let result = verify(&actr_pkg, &key_b.verifying_key());
    assert!(result.is_err(), "verify with wrong key should fail");
}

/// Test 8: key_id format validation
#[test]
fn key_id_format_is_correct() {
    let key = SigningKey::generate(&mut OsRng);
    let key_id = actr_pack::compute_key_id(&key.verifying_key().to_bytes());

    // Must start with "mfr-"
    assert!(key_id.starts_with("mfr-"), "key_id must start with 'mfr-'");
    // Must be "mfr-" + 16 hex chars = 20 chars total
    assert_eq!(key_id.len(), 20, "key_id must be 20 chars (mfr- + 16 hex)");
    // The hex part must be valid hex
    let hex_part = &key_id[4..];
    assert!(
        hex_part.chars().all(|c| c.is_ascii_hexdigit()),
        "key_id suffix must be valid hex"
    );
}

/// Test 9: deterministic — same key always produces same key_id
#[test]
fn compute_key_id_is_deterministic() {
    let key = SigningKey::generate(&mut OsRng);
    let pub_bytes = key.verifying_key().to_bytes();
    let id1 = actr_pack::compute_key_id(&pub_bytes);
    let id2 = actr_pack::compute_key_id(&pub_bytes);
    assert_eq!(id1, id2, "compute_key_id must be deterministic");
}

/// Test 10: different keys produce different key_ids
#[test]
fn different_keys_produce_different_key_ids() {
    let key_a = SigningKey::generate(&mut OsRng);
    let key_b = SigningKey::generate(&mut OsRng);
    let id_a = actr_pack::compute_key_id(&key_a.verifying_key().to_bytes());
    let id_b = actr_pack::compute_key_id(&key_b.verifying_key().to_bytes());
    assert_ne!(
        id_a, id_b,
        "different keys should produce different key_ids"
    );
}
