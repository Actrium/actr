/// Host 在调用 `actr_init` 时传入的 actor 配置
///
/// 由 Host（Hyper 层）序列化为 JSON 写入 WASM 线性内存，guest 反序列化后使用。
#[derive(Debug, serde::Deserialize)]
pub struct ActorConfig {
    /// actor 类型标识，格式：`manufacturer:name:version`
    pub actr_type: String,
    /// base64 编码的凭证（由 AIS 颁发）
    pub credential_b64: String,
    /// base64 编码的 actor 实例 ID
    pub actor_id_b64: String,
    /// 所属 realm ID
    pub realm_id: u32,
}
