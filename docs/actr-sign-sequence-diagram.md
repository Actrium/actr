# Actr Sign & Auth Full Pipeline Sequence Diagrams

> **Core signing logic: developers sign packages with their private key and publish to the platform, which hosts the public key; users download the package and verify the signature with the public key — confirming the package was published by that developer and has not been tampered with.**

## Overview

```mermaid
sequenceDiagram
    autonumber
    participant Dev as Developer
    participant GitHub as GitHub
    participant MFR as MFR
    participant CLI as actr CLI
    participant Hyper as Hyper
    participant AIS as AIS
    participant Signer as Signer
    participant Sig as Signaling

    rect rgb(240, 230, 250)
    Note right of Dev: Phase 1 MFR Registration
    Dev->>MFR: POST /mfr/apply
    MFR-->>Dev: challenge_token + gh CLI command
    Dev->>GitHub: Create repo, write token
    Dev->>MFR: POST /mfr/{id}/verify
    MFR->>GitHub: Verify token
    MFR-->>Dev: mfr-keychain.json
    end

    rect rgb(255, 240, 230)
    Note right of Dev: Phase 2 Build + Phase 3 Publish
    Dev->>CLI: actr pkg build --binary .wasm --key keychain
    CLI->>CLI: SHA-256 binary + proto hash, Ed25519 sign
    CLI-->>Dev: .actr signed package
    Dev->>CLI: actr pkg publish --package .actr
    CLI->>MFR: POST /mfr/pkg/publish with manifest + sig + proto filing
    MFR->>MFR: verify_signature, INSERT mfr_package + proto_files
    end

    rect rgb(230, 250, 230)
    Note right of Hyper: Phase 4 Verify + Phase 5 Register
    Hyper->>Hyper: verify_package .actr with Ed25519 + SHA-256
    Hyper->>AIS: POST /ais/register + manifest_raw + mfr_signature
    AIS->>MFR: lookup_package type_str + target + manifest_hash
    Note over AIS: Path 1 published pkg match, or Path 2 own package sig verify
    AIS->>Signer: gRPC Sign claims
    AIS-->>Hyper: credential AIdCredential
    end

    rect rgb(230, 240, 255)
    Note right of Hyper: Phase 6 Go Online
    Hyper->>Sig: WebSocket + credential
    Sig->>Sig: verify_strict credential
    Sig-->>Hyper: Actor online
    end
```

---

Detailed sequence diagrams for each phase below.

---

## Phase 1: MFR Manufacturer Registration

> One-time operation. Developer verifies GitHub identity to obtain MFR signing keys.
> Supports two key modes: developer uploads own public key (recommended), or platform generates keypair.

```mermaid
sequenceDiagram
    autonumber
    participant Dev as Developer
    participant MFR as MFR Service
    participant GitHub as GitHub

    Dev->>MFR: POST /mfr/apply with github_login
    MFR->>MFR: validate_github_login + generate challenge_token
    MFR-->>Dev: mfr_id, challenge_token, verify_file + gh CLI command
    Dev->>Dev: Execute gh CLI command
    Dev->>GitHub: gh repo create login/actr-mfr-verify --public, write domain.txt with token, push
    Dev->>MFR: POST /mfr/id/verify
    MFR->>GitHub: GET api.github.com/repos/login/actr-mfr-verify/contents/domain.txt
    GitHub-->>MFR: content base64
    MFR->>MFR: Decode and verify token
    alt Uploaded mode - user provides public key
        MFR->>MFR: validate_public_key, store public key
        MFR-->>Dev: mfr-keychain.json with public_key only
    else Generated mode - platform generates keypair
        MFR->>MFR: generate_keypair Ed25519
        MFR-->>Dev: mfr-keychain.json with private_key + public_key
    end
    MFR->>MFR: activate MFR, set key_expires_at
```

**Key source**: `KeySource::Uploaded` (recommended) or `KeySource::Generated`

**Code**: [handlers.rs](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/mfr/src/handlers.rs), [manager.rs verify_github](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/mfr/src/manager.rs), [crypto.rs](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/mfr/src/crypto.rs)

---

## Phase 2: Package Signing (Build)

> Developer signs the Actor binary + proto files into an `.actr` package using the MFR key.

```mermaid
sequenceDiagram
    autonumber
    participant Dev as Developer
    participant CLI as actr CLI
    participant Pack as actr_pack

    Dev->>CLI: actr pkg build --binary actor.wasm --key mfr-keychain.json
    CLI->>CLI: load_signing_key from JSON, base64 decode to SigningKey
    CLI->>CLI: Read actr.toml package section, build PackageManifest
    CLI->>CLI: Read binary file
    CLI->>CLI: Read proto files from exports in actr.toml
    CLI->>Pack: pack with manifest, binary, proto_files, signing_key
    Pack->>Pack: SHA-256 binary_bytes to hash
    Pack->>Pack: SHA-256 each proto file to hash
    Pack->>Pack: manifest.binary.hash = binary_hash
    Pack->>Pack: manifest.proto_files = proto entries with hashes
    Pack->>Pack: manifest.to_toml to manifest_bytes
    Pack->>Pack: signing_key.sign manifest_bytes to actr.sig 64 bytes
    Pack->>Pack: Write ZIP STORE: actr.toml + actr.sig + bin/actor.wasm + proto/*.proto
    Pack-->>CLI: .actr file bytes
    CLI-->>Dev: manufacturer-name-version-target.actr
```

### .actr Package Structure

```text
{mfr}-{name}-{version}-{target}.actr (ZIP STORE)
+-- actr.toml             # manifest (TOML, binary hash + proto hashes)
+-- actr.sig              # Ed25519 signature over actr.toml (64 bytes raw)
+-- bin/actor.wasm        # binary (STORE mode)
+-- proto/echo.proto      # proto file 1 (optional)
+-- proto/common.proto    # proto file 2 (optional)
```

### Signing Chain

```text
binary bytes  --> SHA-256 --> actr.toml[binary.hash]
proto bytes   --> SHA-256 --> actr.toml[proto_files[].hash]
                                   |
                           actr.toml bytes --> Ed25519 sign --> actr.sig
```

**Code**: [pkg.rs execute_build](file:///Users/zhj/RustProject/Actrium/actr/cli/src/commands/pkg.rs), [pack.rs](file:///Users/zhj/RustProject/Actrium/actr/core/pack/src/pack.rs), [fingerprint.rs](file:///Users/zhj/RustProject/Actrium/actr/core/service-compat/src/fingerprint.rs)

---

## Phase 3: Publish to Registry

> Register package metadata + proto filing to MFR so AIS can verify during registration.

```mermaid
sequenceDiagram
    autonumber
    participant Dev as Developer
    participant CLI as actr CLI
    participant MFR as MFR Service

    Dev->>CLI: actr pkg publish --package .actr --keychain mfr.json
    CLI->>CLI: Extract actr.toml from .actr ZIP via read_manifest_raw
    CLI->>CLI: Parse manufacturer/name/version/target
    CLI->>CLI: mfr_key.sign manifest_bytes to signature base64
    CLI->>CLI: Extract proto files via read_proto_files
    CLI->>CLI: Build proto_files JSON from proto files
    CLI->>MFR: POST /mfr/pkg/publish with manufacturer, name, version, target, manifest, signature, proto_files
    MFR->>MFR: Check MFR status == Active
    MFR->>MFR: Check signing_key not expired via key_expires_at
    MFR->>MFR: verify_signature manifest, sig, pubkey
    MFR->>MFR: INSERT mfr_package with type_str, manifest, signature, proto_files, status=active
    MFR-->>CLI: ActrPackage with type_str, id
    CLI-->>Dev: Published: manufacturer:name:version
```

**Code**: [pkg.rs execute_publish](file:///Users/zhj/RustProject/Actrium/actr/cli/src/commands/pkg.rs), [manager.rs publish_package](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/mfr/src/manager.rs)

---

## Phase 4: Runtime Verification

> Hyper layer loads the `.actr` file, fetches the MFR public key, and verifies signature + all hashes (binary + proto + resources).

```mermaid
sequenceDiagram
    autonumber
    participant App as Application
    participant Hyper as Hyper Layer
    participant Verifier as PackageVerifier
    participant Cache as MfrCertCache
    participant AIS as AIS Service
    participant Pack as actr_pack

    App->>Hyper: verify_package actr_bytes
    Hyper->>Pack: read_manifest bytes, extract manufacturer
    alt Production mode
        Hyper->>Verifier: prefetch_mfr_cert manufacturer
        Verifier->>Cache: get_or_fetch manufacturer
        Cache->>Cache: cache miss
        Cache->>AIS: GET /mfr/name/verifying_key
        AIS-->>Cache: public_key base64
        Cache->>Cache: base64 decode to VerifyingKey, store in cache TTL 1h
    else Development mode
        Hyper->>Hyper: Use self_signed_pubkey
    end
    Hyper->>Verifier: verify bytes
    Verifier->>Verifier: resolve_mfr_pubkey manufacturer
    Verifier->>Pack: verify bytes, pubkey
    Pack->>Pack: Read actr.sig to Signature
    Pack->>Pack: Read actr.toml to manifest_bytes
    Pack->>Pack: pubkey.verify_strict manifest_bytes, sig
    Pack->>Pack: SHA-256 binary == manifest.binary.hash
    Pack->>Pack: SHA-256 proto files == manifest.proto_files[].hash
    Pack->>Pack: SHA-256 resources == manifest.resources[].hash
    Pack-->>Hyper: PackageManifest
    Hyper-->>App: manifest verified
```

**Code**: [verify/mod.rs](file:///Users/zhj/RustProject/Actrium/actr/core/hyper/src/verify/mod.rs), [cert_cache.rs](file:///Users/zhj/RustProject/Actrium/actr/core/hyper/src/verify/cert_cache.rs), [verify.rs](file:///Users/zhj/RustProject/Actrium/actr/core/pack/src/verify.rs)

---

## Phase 5: AIS Credential Issuance

> Actor registers with AIS to obtain an identity credential.
> AIS uses a **dual-path** verification:
> - **Path 1**: Published package — lookup in mfr_package table by type_str + target + manifest_hash.
> - **Path 2**: Unpublished package (own package) — if lookup fails, AIS retrieves the manufacturer's public key and verifies the MFR signature on the manifest. If the signature is valid, it proves the manufacturer is running its own package (signed with its own private key), allowing registration without prior publishing.

```mermaid
sequenceDiagram
    autonumber
    participant Hyper as Hyper / ActrNode
    participant AIS as AIS Handler
    participant Issuer as AIdIssuer
    participant MFR as MFR DB
    participant Signer as Signer Service
    participant Validator as CredentialValidator

    Hyper->>AIS: POST /ais/register protobuf + X-Realm-Secret + manifest_raw + mfr_signature + target
    AIS->>AIS: RegisterRequest decode body
    AIS->>AIS: validate_realm realm_id
    AIS->>AIS: verify_realm_secret realm_id, secret
    Note over AIS: NotConfigured / ValidCurrent / ValidPrevious proceed
    AIS->>Issuer: issue_credential request
    Issuer->>Issuer: ensure_key_loaded
    alt Cache empty or key expired
        Issuer->>Signer: gRPC generate_signing_key
        Signer-->>Issuer: key_id, verifying_key, expires_at
        Issuer->>Validator: persist_key to signaling_key_cache.db
    end
    Issuer->>Issuer: verify_mfr_identity request
    alt Path 1: published package
        Issuer->>MFR: lookup_package type_str + target + manifest_hash
        MFR-->>Issuer: found and active, pass
    else Path 2: own package, not yet published
        Note over Issuer: Package not in MFR table, verify manufacturer owns this package
        Issuer->>MFR: resolve_by_name manufacturer, get public key
        Issuer->>Issuer: verify_signature manifest_raw, mfr_signature, mfr_pubkey
        Note over Issuer: If MFR pubkey can verify the sig, the manufacturer signed this package itself
        alt Signature valid - manufacturer is running its own package
            Issuer->>Issuer: pass
        else Signature invalid or no signature provided
            Issuer-->>AIS: Error ManufacturerNotVerified
        end
    end
    Issuer->>Issuer: generate_actr_id Snowflake serial_number
    Issuer->>Issuer: Build IdentityClaims realm_id, actor_id, expires_at
    Issuer->>Issuer: claims.encode_to_vec
    Issuer->>Signer: gRPC Sign key_id, claims_bytes
    Signer->>Signer: AES-256-GCM decrypt private key
    Signer->>Signer: Ed25519 sign claims_bytes
    Signer-->>Issuer: signature 64 bytes
    Issuer->>Issuer: AIdCredential with key_id, claims, signature
    Issuer->>Issuer: generate_turn_credential HMAC-SHA1
    Issuer-->>AIS: RegisterOk
    AIS->>AIS: Persist ACL rules
    AIS->>AIS: Store pending_registration
    AIS-->>Hyper: RegisterOk with actr_id, credential, turn_credential, signing_pubkey
```

**Code**: [handlers.rs](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/ais/src/handlers.rs), [issuer.rs verify_mfr_identity](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/ais/src/issuer.rs), [manager.rs lookup_package](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/mfr/src/manager.rs)

---

## Phase 6: Signaling Connection Auth

> Use the AIS-issued credential to connect to Signaling, verified by the Validator.

```mermaid
sequenceDiagram
    autonumber
    participant Node as ActrNode
    participant SC as SignalingClient
    participant Signaling as Signaling Server
    participant Validator as AIdCredentialValidator
    participant KeyCache as KeyCache SQLite

    Node->>Node: inject_credential register_ok
    Node->>SC: set_actor_id actr_id
    Node->>SC: set_credential_state credential
    SC->>Signaling: WebSocket connect, URL carries credential params
    Signaling->>Validator: check credential, realm_id
    Validator->>KeyCache: get_cached_key key_id
    KeyCache-->>Validator: verifying_key
    Validator->>Validator: AIdCredentialVerifier verify, verify_strict claims_bytes, signature
    Validator->>Validator: claims.realm_id == realm_id
    Validator->>Validator: claims.expires_at > now
    Validator-->>Signaling: IdentityClaims, valid
    Signaling-->>SC: Connection established
    SC-->>Node: Actor online
    Note over Node,Signaling: Subsequent SDP/ICE exchange via Signaling to establish WebRTC P2P connection
```

**Code**: [actr_node.rs](file:///Users/zhj/RustProject/Actrium/actr/core/hyper/src/lifecycle/actr_node.rs), [validator.rs](file:///Users/zhj/RustProject/Actrium/actrix/crates/platform/src/aid/credential/validator.rs)



---

## Known Issues

### 1. PSK renewal not implemented (not yet supported)

AIS always returns `psk: None` when issuing credentials. Every registration goes through the full flow with no lightweight renewal.

**Solution**: Generate an HMAC-SHA256 PSK in `issue_credential` (using `actr_id + actr_type + realm_id + expires_at` as input) and return it with `RegisterOk`. Hyper client uses the PSK to call `/ais/renew` before credential expiry.

### 2. Package distribution logic missing (not yet supported)

`actr pkg publish` only registers metadata (manifest text + signature + proto filing) to MFR — it does not upload the `.actr` file. MFR has no package storage or download capabilities.

**Solution**: Upload the full `.actr` package during publish. MFR verifies the package server-side via `actr_pack::verify()`, then stores it in object storage (S3/MinIO). Add `actr pkg pull <actr_type>` for downloading.
