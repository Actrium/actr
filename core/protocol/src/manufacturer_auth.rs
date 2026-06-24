//! Manufacturer registration proof payload helpers.
//!
//! These helpers define the stable bytes signed by a package launcher using the
//! package manufacturer's key. For unpublished package registration, AIS
//! verifies this signature with the same MFR key that signed the package
//! manifest.

use crate::ActrType;

pub const MANUFACTURER_REGISTER_DOMAIN: &str = "ACTR-MANUFACTURER-REGISTER-V1";

#[derive(Debug, Clone, Copy)]
pub struct ManufacturerRegisterPayload<'a> {
    pub realm_id: u32,
    pub actr_type: &'a ActrType,
    pub target: &'a str,
    pub manifest_sha256_hex: &'a str,
    pub manufacturer_signed_at: u64,
    pub manufacturer_nonce: &'a [u8],
}

pub fn build_manufacturer_register_payload(input: ManufacturerRegisterPayload<'_>) -> String {
    format!(
        "{domain}\n\
         auth_mode=package\n\
         realm={realm}\n\
         actr_type={actr_type}\n\
         target={target}\n\
         manifest_sha256={manifest_sha256}\n\
         manufacturer_signed_at={manufacturer_signed_at}\n\
         manufacturer_nonce={manufacturer_nonce}",
        domain = MANUFACTURER_REGISTER_DOMAIN,
        realm = input.realm_id,
        actr_type = input.actr_type.to_string_repr(),
        target = input.target,
        manifest_sha256 = input.manifest_sha256_hex,
        manufacturer_signed_at = input.manufacturer_signed_at,
        manufacturer_nonce = lower_hex(input.manufacturer_nonce),
    )
}

pub fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manufacturer_register_payload_is_stable() {
        let actr_type = ActrType {
            manufacturer: "acme".to_string(),
            name: "echo".to_string(),
            version: "1.2.3".to_string(),
        };

        let payload = build_manufacturer_register_payload(ManufacturerRegisterPayload {
            realm_id: 7,
            actr_type: &actr_type,
            target: "x86_64-unknown-linux-gnu",
            manifest_sha256_hex: "abc123",
            manufacturer_signed_at: 1_782_200_000,
            manufacturer_nonce: &[0xab, 0xcd],
        });

        assert_eq!(
            payload,
            "ACTR-MANUFACTURER-REGISTER-V1\n\
             auth_mode=package\n\
             realm=7\n\
             actr_type=acme:echo:1.2.3\n\
             target=x86_64-unknown-linux-gnu\n\
             manifest_sha256=abc123\n\
             manufacturer_signed_at=1782200000\n\
             manufacturer_nonce=abcd"
        );
    }
}
