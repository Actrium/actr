# Actr 签名认证全链路时序图

## 阶段一：MFR 制造商注册

> 一次性操作。开发者通过 GitHub 身份验证获得 MFR 签名密钥。

```mermaid
sequenceDiagram
    autonumber
    participant Dev as 开发者
    participant MFR as MFR Service
    participant GitHub as GitHub

    Dev->>MFR: POST /mfr/apply {github_login}
    MFR->>MFR: 生成 challenge_token
    MFR-->>Dev: {mfr_id, challenge_token, verify_file}<br/>+ gh CLI 命令
    Dev->>Dev: 执行 gh CLI 命令
    Dev->>GitHub: gh repo create {login}/actr-mfr-verify --public<br/>写入 {domain}.txt (含 token) → push
    Dev->>MFR: POST /mfr/{id}/verify
    MFR->>GitHub: GET api.github.com/repos/<br/>{login}/actr-mfr-verify/contents/{domain}.txt
    GitHub-->>MFR: {content: base64}
    MFR->>MFR: 解码验证 token → generate_keypair()
    MFR-->>Dev: mfr-keychain.json<br/>{private_key, public_key}
```

**代码位置**: [handlers.rs](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/mfr/src/handlers.rs), [crypto.rs](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/mfr/src/crypto.rs)

---

## 阶段二：打包签名

> 开发者使用密钥将 Actor 二进制打包为 `.actr` 文件。

```mermaid
sequenceDiagram
    autonumber
    participant Dev as 开发者
    participant CLI as actr CLI
    participant Pack as actr_pack

    Dev->>CLI: actr pkg build --binary actor.wasm<br/>--key mfr-keychain.json
    CLI->>CLI: load_signing_key(key.json)<br/>base64 解码 → SigningKey
    CLI->>CLI: 读取 actr.toml [package] 段<br/>构建 PackageManifest
    CLI->>CLI: 读取 binary 文件
    CLI->>Pack: pack(manifest, binary, signing_key)
    Pack->>Pack: SHA-256(binary_bytes) → hash
    Pack->>Pack: manifest.binary.hash = hash
    Pack->>Pack: manifest.to_toml() → manifest_bytes
    Pack->>Pack: signing_key.sign(manifest_bytes)<br/>→ actr.sig (64 bytes)
    Pack->>Pack: 写 ZIP STORE:<br/>actr.toml + actr.sig + binary
    Pack-->>CLI: .actr 文件字节
    CLI-->>Dev: acme-Echo-v1-wasm32.actr
```

**代码位置**: [pkg.rs execute_build](file:///Users/zhj/RustProject/Actrium/actr/cli/src/commands/pkg.rs#L170-L273), [pack.rs](file:///Users/zhj/RustProject/Actrium/actr/core/pack/src/pack.rs#L26-L92)

---

## 阶段三：发布注册

> 将包元数据注册到 MFR，让 AIS 能查到该 actr_type。

```mermaid
sequenceDiagram
    autonumber
    participant Dev as 开发者
    participant CLI as actr CLI
    participant MFR as MFR Service

    Dev->>CLI: actr pkg publish --package .actr<br/>--keychain mfr.json
    CLI->>CLI: 从 .actr ZIP 提取 actr.toml 文本<br/>(read_manifest_raw)
    CLI->>CLI: 解析 manufacturer/name/version
    CLI->>CLI: mfr_key.sign(manifest_bytes)<br/>→ signature (base64)
    CLI->>MFR: POST /mfr/pkg/publish<br/>{manufacturer, name, version,<br/>manifest, signature}
    MFR->>MFR: 查 MFR 表: status == Active?
    MFR->>MFR: 检查 signing_key 未过期
    MFR->>MFR: verify_signature(manifest, sig, pubkey)
    MFR->>MFR: INSERT mfr_package<br/>(type_str, status=active)
    MFR-->>CLI: ActrPackage {type_str, id}
    CLI-->>Dev: ✅ Published: acme:Echo:v1
```

**代码位置**: [pkg.rs execute_publish](file:///Users/zhj/RustProject/Actrium/actr/cli/src/commands/pkg.rs#L380-L472), [manager.rs publish_package](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/mfr/src/manager.rs#L196-L237)

---

## 阶段四：运行时验签

> Hyper 层加载 `.actr` 文件，获取公钥并验证签名和 hash。

```mermaid
sequenceDiagram
    autonumber
    participant App as 应用程序
    participant Hyper as Hyper 层
    participant Verifier as PackageVerifier
    participant Cache as MfrCertCache
    participant AIS as AIS Service
    participant Pack as actr_pack

    App->>Hyper: verify_package(actr_bytes)
    Hyper->>Pack: read_manifest(bytes)<br/>提取 manufacturer
    alt Production 模式
        Hyper->>Verifier: prefetch_mfr_cert(manufacturer)
        Verifier->>Cache: get_or_fetch(manufacturer)
        Cache->>Cache: cache miss
        Cache->>AIS: GET /mfr/{name}/verifying_key
        AIS-->>Cache: {public_key: base64}
        Cache->>Cache: base64 解码 → VerifyingKey<br/>存入缓存 (TTL 1h)
    else Development 模式
        Hyper->>Hyper: 使用 self_signed_pubkey
    end
    Hyper->>Verifier: verify(bytes)
    Verifier->>Verifier: resolve_mfr_pubkey(manufacturer)
    Verifier->>Pack: verify(bytes, pubkey)
    Pack->>Pack: 读 actr.sig → Signature
    Pack->>Pack: 读 actr.toml → manifest_bytes
    Pack->>Pack: pubkey.verify_strict(manifest_bytes, sig)
    Pack->>Pack: SHA-256(binary) == manifest.binary.hash
    Pack->>Pack: SHA-256(resources) == manifest.resources[i].hash
    Pack-->>Hyper: PackageManifest
    Hyper-->>App: manifest (verified)
```

**代码位置**: [verify/mod.rs](file:///Users/zhj/RustProject/Actrium/actr/core/hyper/src/verify/mod.rs#L63-L116), [cert_cache.rs](file:///Users/zhj/RustProject/Actrium/actr/core/hyper/src/verify/cert_cache.rs#L64-L149), [verify.rs](file:///Users/zhj/RustProject/Actrium/actr/core/pack/src/verify.rs#L19-L93)

---

## 阶段五：AIS 注册签发 Credential

> Actor 向 AIS 注册，获取身份凭证用于连接 Signaling。

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
    Note over AIS: NotConfigured/ValidCurrent/<br/>ValidPrevious → 继续
    AIS->>Issuer: issue_credential(request)
    Issuer->>Issuer: ensure_key_loaded()
    alt 缓存为空或密钥过期
        Issuer->>Signer: gRPC generate_signing_key()
        Signer-->>Issuer: {key_id, verifying_key, expires_at}
        Issuer->>Validator: persist_key(key_id, verifying_key)<br/>写入 signaling_key_cache.db
    end
    Issuer->>MFR: lookup_package(actr_type)
    alt 保留名 (acme/self/actrix)
        MFR-->>Issuer: true（直接放行）
    else 非保留名
        MFR->>MFR: 查 mfr_package 表
        MFR-->>Issuer: status == active?
    end
    Issuer->>Issuer: generate_actr_id()<br/>(Snowflake serial_number)
    Issuer->>Issuer: 构建 IdentityClaims<br/>{realm_id, actor_id, expires_at}
    Issuer->>Issuer: claims.encode_to_vec()
    Issuer->>Signer: gRPC Sign(key_id, claims_bytes)
    Signer->>Signer: AES-256-GCM 解密私钥
    Signer->>Signer: Ed25519 sign(claims_bytes)
    Signer-->>Issuer: signature (64 bytes)
    Issuer->>Issuer: AIdCredential {key_id, claims, signature}
    Issuer->>Issuer: generate_turn_credential(HMAC-SHA1)
    Issuer-->>AIS: RegisterOk
    AIS->>AIS: 持久化 ACL 规则
    AIS->>AIS: 存储 pending_registration
    AIS-->>Hyper: RegisterOk {actr_id, credential,<br/>turn_credential, signing_pubkey}
```

**代码位置**: [handlers.rs](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/ais/src/handlers.rs#L54-L244), [issuer.rs](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/ais/src/issuer.rs#L532-L602), [manager.rs lookup_package](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/mfr/src/manager.rs#L317-L327)

---

## 阶段六：Signaling 连接认证

> 使用 AIS 签发的 Credential 连接 Signaling，经 Validator 验证后上线。

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
    SC->>Signaling: WebSocket 连接<br/>URL 携带 credential 参数
    Signaling->>Validator: check(credential, realm_id)
    Validator->>KeyCache: get_cached_key(key_id)
    KeyCache-->>Validator: verifying_key
    Validator->>Validator: AIdCredentialVerifier::verify()<br/>verify_strict(claims_bytes, signature)
    Validator->>Validator: claims.realm_id == realm_id ?
    Validator->>Validator: claims.expires_at > now ?
    Validator-->>Signaling: (IdentityClaims, valid)
    Signaling-->>SC: 连接成功
    SC-->>Node: Actor 在线
    Note over Node,Signaling: 后续通过 Signaling<br/>进行 SDP/ICE 交换<br/>建立 WebRTC P2P 连接
```

**代码位置**: [actr_node.rs](file:///Users/zhj/RustProject/Actrium/actr/core/hyper/src/lifecycle/actr_node.rs), [validator.rs](file:///Users/zhj/RustProject/Actrium/actrix/crates/platform/src/aid/credential/validator.rs)

---

## 源码模式（Source/Native）

> 源码模式不经过阶段一~四（无打包、无发布、无验签），直接从 AIS 注册开始。
> 当前仅保留名（acme/self/actrix）可通过 `lookup_package` 校验。

```mermaid
sequenceDiagram
    autonumber
    participant App as 源码 Actor (cargo run)
    participant AIS as AIS Handler
    participant MFR as MFR lookup
    participant Signer as Signer Service
    participant Signaling as Signaling Server
    participant Validator as CredentialValidator

    App->>App: 加载 actr.toml<br/>(actr_type, realm, signaling)
    App->>App: ActrSystem::new(config)
    App->>AIS: POST /ais/register (protobuf)<br/>+ X-Realm-Secret header
    AIS->>AIS: validate_realm(realm_id)
    AIS->>AIS: verify_realm_secret
    AIS->>MFR: lookup_package(actr_type)
    alt 保留名 (acme/self/actrix)
        MFR-->>AIS: true（放行）
    else 非保留名
        MFR->>MFR: 查 mfr_package 表
        MFR-->>AIS: ❌ 无记录 → 校验失败
        AIS-->>App: Error: ManufacturerNotVerified
    end
    AIS->>Signer: gRPC Sign(key_id, claims_bytes)
    Signer-->>AIS: signature
    AIS-->>App: RegisterOk {actr_id, credential}
    App->>Signaling: WebSocket 连接 (携带 credential)
    Signaling->>Validator: check(credential, realm_id)
    Validator-->>Signaling: valid
    Signaling-->>App: Actor 在线
```

> ⚠️ **已知问题**: 源码模式使用非保留名时，因 `mfr_package` 表无记录，`verify_actr_type()` 校验必定失败，需要为源码模式提供注册通道。
