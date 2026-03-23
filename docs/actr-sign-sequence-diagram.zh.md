# Actr 签名认证全链路时序图

> **签名认证核心逻辑：开发者用私钥对安装包签名并发布到平台，平台托管公钥；用户下载包后，用公钥验签——确认包确实由该开发者发布，且内容未被篡改。**

## 概览

```mermaid
sequenceDiagram
    autonumber
    participant Dev as 开发者
    participant GitHub as GitHub
    participant MFR as MFR
    participant CLI as actr CLI
    participant Hyper as Hyper
    participant AIS as AIS
    participant Signer as Signer
    participant Sig as Signaling

    rect rgb(240, 230, 250)
    Note right of Dev: Phase 1 MFR 注册
    Dev->>MFR: POST /mfr/apply
    MFR-->>Dev: challenge_token + gh CLI 命令
    Dev->>GitHub: 创建仓库，写入 token
    Dev->>MFR: POST /mfr/{id}/verify
    MFR->>GitHub: 验证 token
    MFR-->>Dev: mfr-keychain.json
    end

    rect rgb(255, 240, 230)
    Note right of Dev: Phase 2 打包 + Phase 3 发布
    Dev->>CLI: actr pkg build --binary .wasm --key keychain
    CLI->>CLI: SHA-256 binary + proto hash, Ed25519 签名
    CLI-->>Dev: .actr 签名包
    Dev->>CLI: actr pkg publish --package .actr
    CLI->>MFR: POST /mfr/pkg/publish manifest + sig + proto 备案
    MFR->>MFR: verify_signature, INSERT mfr_package + proto_files
    end

    rect rgb(230, 250, 230)
    Note right of Hyper: Phase 4 验签 + Phase 5 注册
    Hyper->>Hyper: verify_package .actr Ed25519 + SHA-256
    Hyper->>AIS: POST /ais/register + manifest_raw + mfr_signature
    AIS->>MFR: lookup_package type_str + target + manifest_hash
    Note over AIS: 路径1 已发布包匹配 或 路径2 自有包签名验证
    AIS->>Signer: gRPC Sign claims
    AIS-->>Hyper: credential AIdCredential
    end

    rect rgb(230, 240, 255)
    Note right of Hyper: Phase 6 上线
    Hyper->>Sig: WebSocket + credential
    Sig->>Sig: verify_strict credential
    Sig-->>Hyper: Actor 在线
    end
```

---

以下为各阶段详细时序图。

---

## 阶段一：MFR 制造商注册

> 一次性操作。开发者通过 GitHub 身份验证获得 MFR 签名密钥。
> 支持两种密钥模式：开发者上传自有公钥或平台生成密钥对。

```mermaid
sequenceDiagram
    autonumber
    participant Dev as 开发者
    participant MFR as MFR Service
    participant GitHub as GitHub

    Dev->>MFR: POST /mfr/apply github_login
    MFR->>MFR: validate_github_login + 生成 challenge_token
    MFR-->>Dev: mfr_id, challenge_token, verify_file + gh CLI 命令
    Dev->>Dev: 执行 gh CLI 命令
    Dev->>GitHub: gh repo create login/actr-mfr-verify --public, 写入 domain.txt 含 token, push
    Dev->>MFR: POST /mfr/id/verify
    MFR->>GitHub: GET api.github.com/repos/login/actr-mfr-verify/contents/domain.txt
    GitHub-->>MFR: content base64
    MFR->>MFR: 解码验证 token
    alt 上传模式 - 开发者提供公钥
        MFR->>MFR: validate_public_key, 存储公钥
        MFR-->>Dev: mfr-keychain.json 仅含 public_key
    else 生成模式 - 平台生成密钥对
        MFR->>MFR: generate_keypair Ed25519
        MFR-->>Dev: mfr-keychain.json 含 private_key + public_key
    end
    MFR->>MFR: activate MFR, 设置 key_expires_at
```

**密钥来源**: `KeySource::Uploaded`（推荐）或 `KeySource::Generated`

**代码位置**: [handlers.rs](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/mfr/src/handlers.rs), [manager.rs verify_github](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/mfr/src/manager.rs), [crypto.rs](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/mfr/src/crypto.rs)

---

## 阶段二：打包签名（Build）

> 开发者使用 MFR 密钥将 Actor 二进制 + proto 文件打包签名为 `.actr` 文件。

```mermaid
sequenceDiagram
    autonumber
    participant Dev as 开发者
    participant CLI as actr CLI
    participant Pack as actr_pack

    Dev->>CLI: actr pkg build --binary actor.wasm --key mfr-keychain.json
    CLI->>CLI: load_signing_key 从 JSON 加载, base64 解码为 SigningKey
    CLI->>CLI: 读取 actr.toml package 段, 构建 PackageManifest
    CLI->>CLI: 读取 binary 文件
    CLI->>CLI: 从 actr.toml exports 读取 proto 文件
    CLI->>Pack: pack manifest, binary, proto_files, signing_key
    Pack->>Pack: SHA-256 binary_bytes 得到 hash
    Pack->>Pack: SHA-256 每个 proto 文件得到 hash
    Pack->>Pack: manifest.binary.hash = binary_hash
    Pack->>Pack: manifest.proto_files = proto 条目含 hash
    Pack->>Pack: manifest.to_toml 得到 manifest_bytes
    Pack->>Pack: signing_key.sign manifest_bytes 得到 actr.sig 64 字节
    Pack->>Pack: 写 ZIP STORE: actr.toml + actr.sig + bin/actor.wasm + proto/*.proto
    Pack-->>CLI: .actr 文件字节
    CLI-->>Dev: manufacturer-name-version-target.actr
```

### .actr 包结构

```text
{mfr}-{name}-{version}-{target}.actr (ZIP STORE)
+-- actr.toml             # manifest (TOML, 含 binary hash + proto hash)
+-- actr.sig              # Ed25519 签名，覆盖 actr.toml (64 字节原始格式)
+-- bin/actor.wasm        # 二进制 (STORE 模式)
+-- proto/echo.proto      # proto 文件 1 (可选)
+-- proto/common.proto    # proto 文件 2 (可选)
```

### 签名链

```text
binary 字节  --> SHA-256 --> actr.toml[binary.hash]
proto 字节   --> SHA-256 --> actr.toml[proto_files[].hash]
                                |
                        actr.toml 字节 --> Ed25519 签名 --> actr.sig
```

**代码位置**: [pkg.rs execute_build](file:///Users/zhj/RustProject/Actrium/actr/cli/src/commands/pkg.rs), [pack.rs](file:///Users/zhj/RustProject/Actrium/actr/core/pack/src/pack.rs), [fingerprint.rs](file:///Users/zhj/RustProject/Actrium/actr/core/service-compat/src/fingerprint.rs)

---

## 阶段三：发布注册（Publish）

> 将包元数据 + proto 备案信息注册到 MFR，供 AIS 在注册时验证。

```mermaid
sequenceDiagram
    autonumber
    participant Dev as 开发者
    participant CLI as actr CLI
    participant MFR as MFR Service

    Dev->>CLI: actr pkg publish --package .actr --keychain mfr.json
    CLI->>CLI: 从 .actr ZIP 提取 actr.toml read_manifest_raw
    CLI->>CLI: 解析 manufacturer/name/version/target
    CLI->>CLI: mfr_key.sign manifest_bytes 得到 signature base64
    CLI->>CLI: 通过 read_proto_files 提取 proto 文件
    CLI->>CLI: 构建 proto_files JSON
    CLI->>MFR: POST /mfr/pkg/publish manufacturer, name, version, target, manifest, signature, proto_files
    MFR->>MFR: 检查 MFR 表 status == Active
    MFR->>MFR: 检查 signing_key 未过期 key_expires_at
    MFR->>MFR: verify_signature manifest, sig, pubkey
    MFR->>MFR: INSERT mfr_package type_str, manifest, signature, proto_files, status=active
    MFR-->>CLI: ActrPackage type_str, id
    CLI-->>Dev: Published manufacturer:name:version
```

**代码位置**: [pkg.rs execute_publish](file:///Users/zhj/RustProject/Actrium/actr/cli/src/commands/pkg.rs), [manager.rs publish_package](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/mfr/src/manager.rs)

---

## 阶段四：运行时验签

> Hyper 层加载 `.actr` 文件，获取 MFR 公钥并验证签名 + 所有 hash（binary + proto + resources）。

```mermaid
sequenceDiagram
    autonumber
    participant App as 应用程序
    participant Hyper as Hyper 层
    participant Verifier as PackageVerifier
    participant Cache as MfrCertCache
    participant AIS as AIS Service
    participant Pack as actr_pack

    App->>Hyper: verify_package actr_bytes
    Hyper->>Pack: read_manifest bytes, 提取 manufacturer
    alt Production 模式
        Hyper->>Verifier: prefetch_mfr_cert manufacturer
        Verifier->>Cache: get_or_fetch manufacturer
        Cache->>Cache: cache miss
        Cache->>AIS: GET /mfr/name/verifying_key
        AIS-->>Cache: public_key base64
        Cache->>Cache: base64 解码为 VerifyingKey, 存入缓存 TTL 1h
    else Development 模式
        Hyper->>Hyper: 使用 self_signed_pubkey
    end
    Hyper->>Verifier: verify bytes
    Verifier->>Verifier: resolve_mfr_pubkey manufacturer
    Verifier->>Pack: verify bytes, pubkey
    Pack->>Pack: 读 actr.sig 得到 Signature
    Pack->>Pack: 读 actr.toml 得到 manifest_bytes
    Pack->>Pack: pubkey.verify_strict manifest_bytes, sig
    Pack->>Pack: SHA-256 binary == manifest.binary.hash
    Pack->>Pack: SHA-256 proto files == manifest.proto_files[].hash
    Pack->>Pack: SHA-256 resources == manifest.resources[].hash
    Pack-->>Hyper: PackageManifest
    Hyper-->>App: manifest 已验证
```

**代码位置**: [verify/mod.rs](file:///Users/zhj/RustProject/Actrium/actr/core/hyper/src/verify/mod.rs), [cert_cache.rs](file:///Users/zhj/RustProject/Actrium/actr/core/hyper/src/verify/cert_cache.rs), [verify.rs](file:///Users/zhj/RustProject/Actrium/actr/core/pack/src/verify.rs)

---

## 阶段五：AIS 注册签发 Credential

> Actor 向 AIS 注册，获取身份凭证用于连接 Signaling。
> AIS 采用**双路径验证**：
> - **路径 1**：已发布包 — 在 mfr_package 表中按 type_str + target + manifest_hash 查找匹配。
> - **路径 2**：未发布包（自有包）— 查找失败时，AIS 获取制造商公钥并验证 manifest 上的 MFR 签名。签名有效则证明制造商在运行自己签名的包（用自己的私钥签过），允许在未发布的情况下注册。

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
    Note over AIS: NotConfigured / ValidCurrent / ValidPrevious 放行
    AIS->>Issuer: issue_credential request
    Issuer->>Issuer: ensure_key_loaded
    alt 缓存为空或密钥过期
        Issuer->>Signer: gRPC generate_signing_key
        Signer-->>Issuer: key_id, verifying_key, expires_at
        Issuer->>Validator: persist_key 写入 signaling_key_cache.db
    end
    Issuer->>Issuer: verify_mfr_identity request
    alt 路径1: 已发布包
        Issuer->>MFR: lookup_package type_str + target + manifest_hash
        MFR-->>Issuer: 找到且状态 active, 通过
    else 路径2: 自有包, 尚未发布
        Note over Issuer: 包不在 MFR 表中, 验证制造商是否在使用自己的包
        Issuer->>MFR: resolve_by_name manufacturer, 获取公钥
        Issuer->>Issuer: verify_signature manifest_raw, mfr_signature, mfr_pubkey
        Note over Issuer: 如果 MFR 公钥能验签, 证明制造商在运行自己签名的包
        alt 签名有效 - 制造商在使用自有包
            Issuer->>Issuer: 通过
        else 签名无效或未提供签名
            Issuer-->>AIS: Error ManufacturerNotVerified
        end
    end
    Issuer->>Issuer: generate_actr_id Snowflake serial_number
    Issuer->>Issuer: 构建 IdentityClaims realm_id, actor_id, expires_at
    Issuer->>Issuer: claims.encode_to_vec
    Issuer->>Signer: gRPC Sign key_id, claims_bytes
    Signer->>Signer: AES-256-GCM 解密私钥
    Signer->>Signer: Ed25519 sign claims_bytes
    Signer-->>Issuer: signature 64 bytes
    Issuer->>Issuer: AIdCredential key_id, claims, signature
    Issuer->>Issuer: generate_turn_credential HMAC-SHA1
    Issuer-->>AIS: RegisterOk
    AIS->>AIS: 持久化 ACL 规则
    AIS->>AIS: 存储 pending_registration
    AIS-->>Hyper: RegisterOk actr_id, credential, turn_credential, signing_pubkey
```

**代码位置**: [handlers.rs](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/ais/src/handlers.rs), [issuer.rs verify_mfr_identity](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/ais/src/issuer.rs), [manager.rs lookup_package](file:///Users/zhj/RustProject/Actrium/actrix/crates/services/mfr/src/manager.rs)

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
    participant KeyCache as KeyCache SQLite

    Node->>Node: inject_credential register_ok
    Node->>SC: set_actor_id actr_id
    Node->>SC: set_credential_state credential
    SC->>Signaling: WebSocket 连接, URL 携带 credential 参数
    Signaling->>Validator: check credential, realm_id
    Validator->>KeyCache: get_cached_key key_id
    KeyCache-->>Validator: verifying_key
    Validator->>Validator: AIdCredentialVerifier verify, verify_strict claims_bytes, signature
    Validator->>Validator: claims.realm_id == realm_id
    Validator->>Validator: claims.expires_at > now
    Validator-->>Signaling: IdentityClaims, valid
    Signaling-->>SC: 连接成功
    SC-->>Node: Actor 在线
    Note over Node,Signaling: 后续通过 Signaling 进行 SDP/ICE 交换建立 WebRTC P2P 连接
```

**代码位置**: [actr_node.rs](file:///Users/zhj/RustProject/Actrium/actr/core/hyper/src/lifecycle/actr_node.rs), [validator.rs](file:///Users/zhj/RustProject/Actrium/actrix/crates/platform/src/aid/credential/validator.rs)

---



## 已知问题

### 1. PSK 续期（暂不支持）

AIS 签发 credential 时始终返回 `psk: None`，每次注册都走完整流程，无法轻量续期。

**解决方案**: AIS `issue_credential` 时生成 HMAC-SHA256 PSK（使用 `actr_id + actr_type + realm_id + expires_at` 作为输入），随 `RegisterOk` 返回。Hyper 客户端在 credential 过期前使用 PSK 调用 `/ais/renew` 接口续期。

### 2. 包分发逻辑（暂不支持）

`actr pkg publish` 只向 MFR 注册元数据（manifest 文本 + signature + proto 备案），不上传 `.actr` 文件。MFR 没有包存储和下载能力。

**解决方案**: `publish` 改为上传整个 `.actr` 包。MFR 服务端通过 `actr_pack::verify()` 完整验证后存储到对象存储（S3/MinIO）。新增 `actr pkg pull <actr_type>` 命令用于下载。
