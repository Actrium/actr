# Actr Sign & Auth Full Pipeline Sequence Diagrams

## Overview

### Wasm / Dynclib Mode

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
    MFR-->>Dev: mfr-keychain.json (Ed25519 key pair)
    end

    rect rgb(255, 240, 230)
    Note right of Dev: Phase 2 Build + Phase 3 Publish
    Dev->>CLI: actr pkg build --binary .wasm --key keychain
    CLI->>CLI: SHA-256(binary) -> manifest -> Ed25519 sign
    CLI-->>Dev: .actr signed package
    Dev->>CLI: actr pkg publish --package .actr
    CLI->>MFR: POST /mfr/pkg/publish (manifest + sig)
    MFR->>MFR: verify_signature -> INSERT mfr_package
    end

    rect rgb(230, 250, 230)
    Note right of Hyper: Phase 4 Verify + Phase 5 Register
    Hyper->>Hyper: verify_package(.actr)<br/>Ed25519 verify_strict + SHA-256 hash
    Hyper->>AIS: POST /ais/register + Realm Secret
    AIS->>MFR: lookup_package(actr_type)
    AIS->>Signer: gRPC Sign(claims)
    AIS-->>Hyper: credential (AIdCredential)
    end

    rect rgb(230, 240, 255)
    Note right of Hyper: Phase 6 Go Online
    Hyper->>Sig: WebSocket + credential
    Sig->>Sig: verify_strict(credential)
    Sig-->>Hyper: Actor online
    end
```

---

### Native Mode

```mermaid
sequenceDiagram
    autonumber
    participant App as Source Actor (cargo run)
    participant AIS as AIS
    participant MFR as MFR
    participant Signer as Signer
    participant Sig as Signaling

    App->>App: Load actr.toml<br/>ActrSystem::new(config)
    App->>AIS: POST /ais/register + Realm Secret
    AIS->>AIS: validate_realm + verify_secret
    AIS->>MFR: lookup_package(actr_type)
    alt Reserved name (acme/self/actrix)
        MFR-->>AIS: ok bypass
    else Non-reserved name
        MFR-->>AIS: no record
        AIS-->>App: Error: ManufacturerNotVerified
    end
    AIS->>Signer: gRPC Sign(claims)
    AIS-->>App: credential (AIdCredential)
    App->>Sig: WebSocket + credential
    Sig->>Sig: verify_strict(credential)
    Sig-->>App: Actor online
```

---

Detailed sequence diagrams for each phase below.

---

## Wasm / Dynclib Mode

### Phase 1: MFR Manufacturer Registration

> One-time operation. Developer verifies GitHub identity to obtain MFR signing keys.

```mermaid
sequenceDiagram
    autonumber
    participant Dev as Developer
    participant MFR as MFR Service
    participant GitHub as GitHub

    Dev->>MFR: POST /mfr/apply {github_login}
    MFR->>MFR: Generate challenge_token
    MFR-->>Dev: {mfr_id, challenge_token, verify_file}<br/>+ gh CLI command
    Dev->>Dev: Execute gh CLI command
    Dev->>GitHub: gh repo create {login}/actr-mfr-verify --public<br/>write {domain}.txt (with token) → push
    Dev->>MFR: POST /mfr/{id}/verify
    MFR->>GitHub: GET api.github.com/repos/<br/>{login}/actr-mfr-verify/contents/{domain}.txt
    GitHub-->>MFR: {content: base64}
    MFR->>MFR: Decode & verify token → generate_keypair()
    MFR-->>Dev: mfr-keychain.json<br/>{private_key, public_key}
```

**Code**: [handlers.rs](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/mfr/src/handlers.rs), [crypto.rs](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/mfr/src/crypto.rs)

---

### Phase 2: Package Signing

> Developer signs the Actor binary into an `.actr` package using the MFR key.

```mermaid
sequenceDiagram
    autonumber
    participant Dev as Developer
    participant CLI as actr CLI
    participant Pack as actr_pack

    Dev->>CLI: actr pkg build --binary actor.wasm<br/>--key mfr-keychain.json
    CLI->>CLI: load_signing_key(key.json)<br/>base64 decode → SigningKey
    CLI->>CLI: Read actr.toml [package] section<br/>build PackageManifest
    CLI->>CLI: Read binary file
    CLI->>Pack: pack(manifest, binary, signing_key)
    Pack->>Pack: SHA-256(binary_bytes) → hash
    Pack->>Pack: manifest.binary.hash = hash
    Pack->>Pack: manifest.to_toml() → manifest_bytes
    Pack->>Pack: signing_key.sign(manifest_bytes)<br/>→ actr.sig (64 bytes)
    Pack->>Pack: Write ZIP STORE:<br/>actr.toml + actr.sig + binary
    Pack-->>CLI: .actr file bytes
    CLI-->>Dev: acme-Echo-v1-wasm32.actr
```

**Code**: [pkg.rs execute_build](file:///Users/zhj/RustProject/Actrium/actr/cli/src/commands/pkg.rs#L170-L273), [pack.rs](file:///Users/zhj/RustProject/Actrium/actr/core/pack/src/pack.rs#L26-L92)

---

### Phase 3: Publish to Registry

> Register package metadata to MFR so AIS can verify the actr_type exists.

```mermaid
sequenceDiagram
    autonumber
    participant Dev as Developer
    participant CLI as actr CLI
    participant MFR as MFR Service

    Dev->>CLI: actr pkg publish --package .actr<br/>--keychain mfr.json
    CLI->>CLI: Extract actr.toml from .actr ZIP<br/>(read_manifest_raw)
    CLI->>CLI: Parse manufacturer/name/version
    CLI->>CLI: mfr_key.sign(manifest_bytes)<br/>→ signature (base64)
    CLI->>MFR: POST /mfr/pkg/publish<br/>{manufacturer, name, version,<br/>manifest, signature}
    MFR->>MFR: Check MFR status == Active
    MFR->>MFR: Check signing_key not expired
    MFR->>MFR: verify_signature(manifest, sig, pubkey)
    MFR->>MFR: INSERT mfr_package<br/>(type_str, status=active)
    MFR-->>CLI: ActrPackage {type_str, id}
    CLI-->>Dev: ✅ Published: acme:Echo:v1
```

**Code**: [pkg.rs execute_publish](file:///Users/zhj/RustProject/Actrium/actr/cli/src/commands/pkg.rs#L380-L472), [manager.rs publish_package](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/mfr/src/manager.rs#L196-L237)

---

### Phase 4: Runtime Verification

> Hyper layer loads the `.actr` file, fetches the public key, and verifies signature + hash.

```mermaid
sequenceDiagram
    autonumber
    participant App as Application
    participant Hyper as Hyper Layer
    participant Verifier as PackageVerifier
    participant Cache as MfrCertCache
    participant AIS as AIS Service
    participant Pack as actr_pack

    App->>Hyper: verify_package(actr_bytes)
    Hyper->>Pack: read_manifest(bytes)<br/>extract manufacturer
    alt Production mode
        Hyper->>Verifier: prefetch_mfr_cert(manufacturer)
        Verifier->>Cache: get_or_fetch(manufacturer)
        Cache->>Cache: cache miss
        Cache->>AIS: GET /mfr/{name}/verifying_key
        AIS-->>Cache: {public_key: base64}
        Cache->>Cache: base64 decode → VerifyingKey<br/>store in cache (TTL 1h)
    else Development mode
        Hyper->>Hyper: Use self_signed_pubkey
    end
    Hyper->>Verifier: verify(bytes)
    Verifier->>Verifier: resolve_mfr_pubkey(manufacturer)
    Verifier->>Pack: verify(bytes, pubkey)
    Pack->>Pack: Read actr.sig → Signature
    Pack->>Pack: Read actr.toml → manifest_bytes
    Pack->>Pack: pubkey.verify_strict(manifest_bytes, sig)
    Pack->>Pack: SHA-256(binary) == manifest.binary.hash
    Pack->>Pack: SHA-256(resources) == manifest.resources[i].hash
    Pack-->>Hyper: PackageManifest
    Hyper-->>App: manifest (verified)
```

**Code**: [verify/mod.rs](file:///Users/zhj/RustProject/Actrium/actr/core/hyper/src/verify/mod.rs#L63-L116), [cert_cache.rs](file:///Users/zhj/RustProject/Actrium/actr/core/hyper/src/verify/cert_cache.rs#L64-L149), [verify.rs](file:///Users/zhj/RustProject/Actrium/actr/core/pack/src/verify.rs#L19-L93)

---

### Phase 5: AIS Credential Issuance

> Actor registers with AIS to obtain an identity credential for Signaling connection.

```mermaid
sequenceDiagram
    autonumber
    participant Hyper as Hyper / ActrNode
    participant AIS as AIS Handler
    participant Issuer as AIdIssuer
    participant MFR as MFR lookup
    participant Signer as Signer Service
    participant Validator as CredentialValidator

    Hyper->>AIS: POST /ais/register (protobuf)<br/>+ X-Realm-Secret header
    AIS->>AIS: RegisterRequest::decode(body)
    AIS->>AIS: validate_realm(realm_id)
    AIS->>AIS: verify_realm_secret(realm_id, secret)
    Note over AIS: NotConfigured/ValidCurrent/<br/>ValidPrevious → proceed
    AIS->>Issuer: issue_credential(request)
    Issuer->>Issuer: ensure_key_loaded()
    alt Cache empty or key expired
        Issuer->>Signer: gRPC generate_signing_key()
        Signer-->>Issuer: {key_id, verifying_key, expires_at}
        Issuer->>Validator: persist_key(key_id, verifying_key)<br/>write to signaling_key_cache.db
    end
    Issuer->>MFR: lookup_package(actr_type)
    alt Reserved name (acme/self/actrix)
        MFR-->>Issuer: true (bypass)
    else Non-reserved name
        MFR->>MFR: Query mfr_package table
        MFR-->>Issuer: status == active?
    end
    Issuer->>Issuer: generate_actr_id()<br/>(Snowflake serial_number)
    Issuer->>Issuer: Build IdentityClaims<br/>{realm_id, actor_id, expires_at}
    Issuer->>Issuer: claims.encode_to_vec()
    Issuer->>Signer: gRPC Sign(key_id, claims_bytes)
    Signer->>Signer: AES-256-GCM decrypt private key
    Signer->>Signer: Ed25519 sign(claims_bytes)
    Signer-->>Issuer: signature (64 bytes)
    Issuer->>Issuer: AIdCredential {key_id, claims, signature}
    Issuer->>Issuer: generate_turn_credential(HMAC-SHA1)
    Issuer-->>AIS: RegisterOk
    AIS->>AIS: Persist ACL rules
    AIS->>AIS: Store pending_registration
    AIS-->>Hyper: RegisterOk {actr_id, credential,<br/>turn_credential, signing_pubkey}
```

**Code**: [handlers.rs](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/ais/src/handlers.rs#L54-L244), [issuer.rs](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/ais/src/issuer.rs#L532-L602), [manager.rs lookup_package](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/mfr/src/manager.rs#L317-L327)

---

### Phase 6: Signaling Connection Auth

> Use the AIS-issued credential to connect to Signaling, verified by the Validator.

```mermaid
sequenceDiagram
    autonumber
    participant Node as ActrNode
    participant SC as SignalingClient
    participant Signaling as Signaling Server
    participant Validator as AIdCredentialValidator
    participant KeyCache as KeyCache (SQLite)

    Node->>Node: inject_credential(register_ok)
    Node->>SC: set_actor_id(actr_id)
    Node->>SC: set_credential_state(credential)
    SC->>Signaling: WebSocket connect<br/>URL carries credential params
    Signaling->>Validator: check(credential, realm_id)
    Validator->>KeyCache: get_cached_key(key_id)
    KeyCache-->>Validator: verifying_key
    Validator->>Validator: AIdCredentialVerifier::verify()<br/>verify_strict(claims_bytes, signature)
    Validator->>Validator: claims.realm_id == realm_id ?
    Validator->>Validator: claims.expires_at > now ?
    Validator-->>Signaling: (IdentityClaims, valid)
    Signaling-->>SC: Connection established
    SC-->>Node: Actor online
    Note over Node,Signaling: Subsequent SDP/ICE exchange<br/>via Signaling to establish<br/>WebRTC P2P connection
```

**Code**: [actr_node.rs](file:///Users/zhj/RustProject/Actrium/actr/core/hyper/src/lifecycle/actr_node.rs), [validator.rs](file:///Users/zhj/RustProject/Actrium/actrix/crates/platform/src/aid/credential/validator.rs)

---

## Native Mode

> Source mode skips Phases 1–4 (no packaging, no publishing, no verification), going directly to AIS registration.
> Currently only reserved names (acme/self/actrix) can pass the `lookup_package` check.

```mermaid
sequenceDiagram
    autonumber
    participant App as Source Actor (cargo run)
    participant AIS as AIS Handler
    participant MFR as MFR lookup
    participant Signer as Signer Service
    participant Signaling as Signaling Server
    participant Validator as CredentialValidator

    App->>App: Load actr.toml<br/>(actr_type, realm, signaling)
    App->>App: ActrSystem::new(config)
    App->>AIS: POST /ais/register (protobuf)<br/>+ X-Realm-Secret header
    AIS->>AIS: validate_realm(realm_id)
    AIS->>AIS: verify_realm_secret
    AIS->>MFR: lookup_package(actr_type)
    alt Reserved name (acme/self/actrix)
        MFR-->>AIS: true (bypass)
    else Non-reserved name
        MFR->>MFR: Query mfr_package table
        MFR-->>AIS: ❌ No record → check failed
        AIS-->>App: Error: ManufacturerNotVerified
    end
    AIS->>Signer: gRPC Sign(key_id, claims_bytes)
    Signer-->>AIS: signature
    AIS-->>App: RegisterOk {actr_id, credential}
    App->>Signaling: WebSocket connect (with credential)
    Signaling->>Validator: check(credential, realm_id)
    Validator-->>Signaling: valid
    Signaling-->>App: Actor online
```

> ⚠️ **Known issue**: Source mode actors using non-reserved manufacturer names will always fail `verify_actr_type()` because there is no `mfr_package` record. A separate registration channel for source mode is needed.
