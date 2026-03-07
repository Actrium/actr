# Realm 全流程梳理与补漏记录（2026-02-24）

## 1. 目标与范围

基于 `TODO-cross-realm.md` 对项目做 realm 全流程审计，重点覆盖：
- realm 元数据管理（control/platform）
- signaling 中的 ACL 持久化与判定
- discovery / relay / route-candidates / presence 的跨 realm 行为
- admin realm secret 分配与轮转、AIS 校验、signaling 透传

本次目标是“治本可落地项”，并明确跨仓库/协议层剩余事项。

## 2. 计划

1. 梳理链路，确认 TODO 与现状偏差。  
2. 修复本仓库可控缺口（ACL 维度、判定一致性、重注册脏数据）。  
3. 增加针对性测试并跑完整相关包测试。  
4. 形成实施与复盘记录。

## 3. Realm 全流程现状梳理

### 3.1 control / platform（realm 数据面）

- realm 主表：`platform/storage/db.rs` -> `realm`
- realm 元数据：`realmconfig`（enabled/use_servers/version）
- 对外管理：`control/src/service.rs` (`create/get/update/delete/list realm`)

### 3.2 signaling（访问控制与数据路径）

- 服务注册时接收 `RegisterRequest.acl`，落地到 `actoracl`。
- discovery / relay / route-candidates / presence 通过 `ActorAcl::can_discover` 决策。

### 3.3 admin + realm secret（鉴权链路）

- realm secret 存储于 `realmconfig`，字段：
  - `realm_secret_hash`
  - `realm_secret_prev_hash`
  - `realm_secret_prev_valid_until`
- admin create realm：
  - 新建 realm 后自动分配 secret（明文仅返回一次）。
- admin rotate realm secret：
  - 立即切换新 secret，并保留旧 secret 兼容窗口（默认 36h）。
- signaling：
  - 从 WebSocket query 读取 `realm_secret`，透传到 AIS 请求头 `x-actrix-realm-secret`。
- AIS：
  - 注册前先校验 realm 生命周期（存在/状态/过期）。
  - 再校验 realm secret（未配置兼容、配置后强制校验）。

## 4. 发现的问题（审计结论）

1. `actoracl` 无来源 realm 维度，跨 realm 无法被规则精确表达。  
2. `service_registry` 某路径仅使用 `type.name` 做匹配，和持久化规则 `manufacturer:name` 不一致。  
3. signaling 多处存在“跨 realm 硬拒绝”，无法与 ACL 放行协同。  
4. 服务重注册时 ACL 会追加写入，旧规则可能残留。

## 5. 实施内容

### 5.1 ACL 模型升级（platform）

- `actoracl` 新增列：`source_realm_id`（可空）。
- 兼容迁移：
  - 若旧表无该列，自动 `ALTER TABLE` 增列。
  - 历史数据回填 `source_realm_id = realm_id`，保留原“同 realm”语义。
- `ActorAcl` 新增能力：
  - `new_with_source_realm(...)`
  - `delete_by_target(realm_id, to_type)`
  - `can_discover(source_realm_id, target_realm_id, from_type, to_type)`

### 5.2 signaling 判定统一（server/service_registry/presence）

- 去除跨 realm 硬编码 deny，统一改为 ACL 判定。
- 全链路统一使用完整类型键：`manufacturer:name`。
- 注册写 ACL 时读取 `principal.realm`：
  - 指定则写入对应 `source_realm_id`
  - 未指定则按同 realm 写入（安全默认）
- 注册前清理同目标类型旧 ACL，避免脏规则累积。

### 5.3 realm secret 全链路补齐（platform/control/actrixd/ais/signaling/admin-web）

- `platform`：
  - 新增 `realm::secret` 模块（分配/轮转/校验/哈希/常量头名）。
- `control`：
  - `create_realm_with_secret_direct`：创建时返回一次性明文 secret。
  - `rotate_realm_secret_direct`：轮转并返回新 secret + 旧 secret 有效期。
- `actrixd admin_api`：
  - `POST /admin/api/realms` 响应新增 `realm_secret`。
  - `POST /admin/api/realms/{id}/secret/rotate` 新增轮转接口。
- `admin web`：
  - Realms 页面支持创建后展示 secret、一键复制、轮转 secret 后展示一次。
- `signaling + ais_client`：
  - 支持透传 `realm_secret` 到 AIS。
  - 修复 AIS 业务错误透传：不再把 AIS protobuf 业务错误吞成 signaling 500。
- `ais`：
  - 注册入口接入 realm lifecycle + realm secret 校验，错误码按 403/500 分类返回。

## 6. 测试

### 6.1 新增用例

- `platform::realm::acl::tests::test_cross_realm_acl_allow_and_deny`
- `signaling::service_registry::tests::test_check_discovery_acl_cross_realm_and_full_type`
- `platform::realm::secret::tests::test_allocate_and_verify_secret`
- `platform::realm::secret::tests::test_rotate_secret_keeps_previous_temporarily_valid`
- `actrix_fullstack::signaling_register_enforces_realm_secret_when_configured`
- `actrix_fullstack::signaling_discovery_cross_realm_acl_allow`
- `actrix_fullstack::signaling_route_candidates_cross_realm_acl_allow`

### 6.2 执行记录

- `cargo test -p platform -- --nocapture`  
  - 结果：`70 passed, 0 failed`
- `cargo test -p signaling -- --nocapture`  
  - 结果：`64 passed, 0 failed`（unit）
  - 结果：`3 passed, 0 failed`（integration）
- `cargo test -p ais -- --nocapture`
  - 结果：`15 passed, 0 failed`（unit+integration）
  - doctest：`2 passed, 0 failed`
- `cargo test -p admin --lib -- --nocapture`
  - 结果：`15 passed, 0 failed`
- `cargo test -p actrix --tests -- --nocapture`
  - 结果：`actrix_fullstack 42 passed, actrix_process 34 passed, actrix_cli 11 passed`
- `npm run build`（`crates/actrixd/admin/web`）
  - 结果：TypeScript + Vite 构建成功

## 7. 复盘

### 7.1 本次已解决

1. 跨 realm 已可被 ACL 精确控制（按 source->target realm + type）。  
2. ACL 类型匹配口径不一致问题已修复。  
3. 重注册 ACL 残留问题已消除。  
4. realm secret 生命周期（分配/轮转/旧密钥兼容窗口/校验）已打通。  
5. admin -> signaling -> AIS 的 secret 鉴权链路已端到端验证。  

### 7.2 仍需推进（非本次可完整闭环）

1. “全 realm 通配”语义仍需协议层明确约定。  
2. 依赖意图检查（`ServiceSpec.dependencies`）尚未接入判定链。  
3. Admin UI 的跨 realm ACL 可视化编辑与审计展示待补。  
4. 需进一步明确 gRPC 管控面下 realm secret 发放/轮转的统一接口形态（当前 Admin UI REST 已覆盖）。
