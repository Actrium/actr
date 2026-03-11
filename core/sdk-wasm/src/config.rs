/// Actor configuration passed by the Host when calling `actr_init`.
///
/// Serialized as JSON into WASM linear memory by the Host (Hyper layer);
/// the guest deserializes it before use.
#[derive(Debug, serde::Deserialize)]
pub struct ActorConfig {
    /// Actor type identifier, format: `manufacturer:name:version`
    pub actr_type: String,
    /// Base64-encoded credential (issued by AIS)
    pub credential_b64: String,
    /// Base64-encoded actor instance ID
    pub actor_id_b64: String,
    /// Realm ID this actor belongs to
    pub realm_id: u32,
}
