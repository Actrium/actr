# 跨 Realm 通信支持：状态与待办（2026-02-24）

## 背景

目标：在保持默认安全（deny by default）的前提下，为 signaling 提供**可控跨 realm 放行**能力。

## 本次已完成

1. ACL 引入来源 realm 维度（已落地）
   - 方案：在 `actoracl` 表新增 `source_realm_id`。
   - 兼容：历史数据自动回填为 `source_realm_id = realm_id`，保留“仅同 realm”语义。
   - 代码：
     - `crates/platform/src/storage/db.rs`
     - `crates/platform/src/realm/acl.rs`

2. signaling 跨 realm 判定从硬拒绝改为 ACL 判定（已落地）
   - relay / discovery / route-candidates / presence / registry 统一走：
     - `ActorAcl::can_discover(source_realm, target_realm, from_type, to_type)`
   - 代码：
     - `crates/signaling/src/server.rs`
     - `crates/signaling/src/service_registry.rs`
     - `crates/signaling/src/presence.rs`

3. 类型匹配一致性修复（已落地）
   - 统一使用 `manufacturer:name`，修复部分路径只用 `name` 导致 ACL 误判的问题。

4. ACL 重注册脏规则清理（已落地）
   - 服务注册时先清理 `realm + to_type` 的旧规则，再写入新规则。
   - 避免历史 ACL 残留造成策略漂移。

## 当前语义（重要）

1. 默认语义：
   - 未命中规则 => deny。
   - 未指定 `principal.realm` => 默认按同 realm 写入（安全优先）。

2. 跨 realm 放行：
   - 在 ACL principal 中显式指定 `realm`，即可放行该来源 realm。
   - 可通过多条 principal 表达“多个来源 realm 白名单”。

## 剩余待办

1. DB 查询语义修复（`platform/src/realm/acl.rs`）
   - `get_by_types` 使用 `ORDER BY rowid DESC LIMIT 1`，语义是”最新行优先”，与 deny-first 评估不符
   - `AND source_realm_id = ?` 无法匹配 NULL 行（wildcard `*` 存为 NULL），需改为
     `AND (source_realm_id IS NULL OR source_realm_id = ?)`
   - 建议：整个 ACL 查询下沉到 DB 层做 JOIN，消除当前 N+1 逐候选查询的性能问题

2. 消费端依赖意图检查（`ServiceSpec.dependencies`）
   - 当前仍未将”消费者声明依赖目标类型”作为第二道 gate
   - 需补齐并与 ACL 判定组合

3. Admin 可视化配置
   - Realm / ACL 管理页尚未支持跨 realm 关系编辑与审计展示

## 测试状态

已覆盖并通过：
- `cargo test -p platform -- --nocapture`
- `cargo test -p signaling -- --nocapture`

新增用例：
- `platform::realm::acl::tests::test_cross_realm_acl_allow_and_deny`
- `signaling::service_registry::tests::test_check_discovery_acl_cross_realm_and_full_type`
