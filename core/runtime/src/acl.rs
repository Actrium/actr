//! ACL (Access Control List) permission checking
//!
//! Starting from the caller_id of an inbound message and the target actor_id,
//! determines whether the call is permitted based on configured ACL rules.
//! This module is pure functions with no IO dependencies, suitable for
//! both native and wasm32 targets.

use actr_protocol::{Acl, AclRule, ActrId, ActrIdExt as _};

/// Check whether the caller has permission to access the target Actor
///
/// # Returns
/// - `Ok(true)`: allowed
/// - `Ok(false)`: denied
/// - `Err(String)`: check error (should be treated as denied)
///
/// # Evaluation logic
/// 1. No caller_id (local call) -- always allow
/// 2. No ACL configured -- allow by default (backward compatibility)
/// 3. ACL configured but rules list empty -- deny all (secure default)
/// 4. Deny-first: any matching DENY rule immediately denies
/// 5. At least one matching ALLOW rule -- allow
/// 6. No rule matches -- deny
pub fn check_acl_permission(
    caller_id: Option<&ActrId>,
    target_id: &ActrId,
    acl: Option<&Acl>,
) -> Result<bool, String> {
    // 1. Local calls are always allowed
    if caller_id.is_none() {
        tracing::trace!("ACL: local call, allowing");
        return Ok(true);
    }

    let caller = caller_id.unwrap();

    // 2. No ACL configured -- allow by default
    let acl = match acl {
        Some(a) => a,
        None => {
            tracing::trace!(
                "ACL: no ACL configured, allowing {} -> {}",
                caller.to_string_repr(),
                target_id.to_string_repr(),
            );
            return Ok(true);
        }
    };

    // 3. Empty rules list -- deny all
    if acl.rules.is_empty() {
        tracing::warn!(
            "ACL: empty rule set, denying {} -> {} (default deny)",
            caller.to_string_repr(),
            target_id.to_string_repr(),
        );
        return Ok(false);
    }

    // 4 & 5. Deny-first evaluation
    let mut any_allow = false;
    for rule in &acl.rules {
        if !matches_rule(caller, rule) {
            continue;
        }
        let is_allow = rule.permission == actr_protocol::acl_rule::Permission::Allow as i32;
        if !is_allow {
            tracing::debug!(
                "ACL: DENY rule matched for {} -> {}",
                caller.to_string_repr(),
                target_id.to_string_repr(),
            );
            return Ok(false);
        }
        any_allow = true;
    }

    if any_allow {
        tracing::debug!(
            "ACL: ALLOW rule matched for {} -> {}",
            caller.to_string_repr(),
            target_id.to_string_repr(),
        );
        return Ok(true);
    }

    // 6. No rule matches -- deny
    tracing::warn!(
        "ACL: no matching rule, denying {} -> {} (default deny)",
        caller.to_string_repr(),
        target_id.to_string_repr(),
    );
    Ok(false)
}

/// Check whether a single ACL rule matches the given caller
fn matches_rule(caller: &ActrId, rule: &AclRule) -> bool {
    use actr_protocol::acl_rule::SourceRealm;

    // Exact type match (manufacturer + name + version)
    if caller.r#type.manufacturer != rule.from_type.manufacturer
        || caller.r#type.name != rule.from_type.name
        || caller.r#type.version != rule.from_type.version
    {
        return false;
    }

    // Realm match
    match &rule.source_realm {
        None | Some(SourceRealm::AnyRealm(_)) => true,
        Some(SourceRealm::RealmId(id)) => caller.realm.realm_id == *id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actr_protocol::{ActrType, Realm, acl_rule::Permission, acl_rule::SourceRealm};

    fn make_id(manufacturer: &str, name: &str, version: &str, realm_id: u32) -> ActrId {
        ActrId {
            serial_number: 0xaabb,
            r#type: ActrType {
                manufacturer: manufacturer.into(),
                name: name.into(),
                version: version.into(),
            },
            realm: Realm { realm_id },
        }
    }

    fn make_rule(manufacturer: &str, name: &str, version: &str, perm: Permission) -> AclRule {
        AclRule {
            permission: perm as i32,
            from_type: ActrType {
                manufacturer: manufacturer.into(),
                name: name.into(),
                version: version.into(),
            },
            source_realm: None,
        }
    }

    #[test]
    fn local_call_always_allowed() {
        let target = make_id("acme", "svc", "0.1.0", 1);
        assert!(check_acl_permission(None, &target, None).unwrap());
    }

    #[test]
    fn no_acl_allows_by_default() {
        let caller = make_id("acme", "client", "0.1.0", 1);
        let target = make_id("acme", "svc", "0.1.0", 1);
        assert!(check_acl_permission(Some(&caller), &target, None).unwrap());
    }

    #[test]
    fn empty_rules_denies() {
        let caller = make_id("acme", "client", "0.1.0", 1);
        let target = make_id("acme", "svc", "0.1.0", 1);
        let acl = Acl { rules: vec![] };
        assert!(!check_acl_permission(Some(&caller), &target, Some(&acl)).unwrap());
    }

    #[test]
    fn deny_overrides_allow() {
        let caller = make_id("acme", "client", "0.1.0", 1);
        let target = make_id("acme", "svc", "0.1.0", 1);
        let acl = Acl {
            rules: vec![
                make_rule("acme", "client", "0.1.0", Permission::Allow),
                make_rule("acme", "client", "0.1.0", Permission::Deny),
            ],
        };
        assert!(!check_acl_permission(Some(&caller), &target, Some(&acl)).unwrap());
    }

    #[test]
    fn allow_when_matched() {
        let caller = make_id("acme", "client", "0.1.0", 1);
        let target = make_id("acme", "svc", "0.1.0", 1);
        let acl = Acl {
            rules: vec![make_rule("acme", "client", "0.1.0", Permission::Allow)],
        };
        assert!(check_acl_permission(Some(&caller), &target, Some(&acl)).unwrap());
    }

    #[test]
    fn no_match_denies() {
        let caller = make_id("acme", "client", "0.1.0", 1);
        let target = make_id("acme", "svc", "0.1.0", 1);
        let acl = Acl {
            rules: vec![make_rule("other", "other", "0.1.0", Permission::Allow)],
        };
        assert!(!check_acl_permission(Some(&caller), &target, Some(&acl)).unwrap());
    }

    #[test]
    fn any_realm_rule_matches_foreign_realm() {
        let caller = make_id("acme", "client", "0.1.0", 2002);
        let target = make_id("acme", "svc", "0.1.0", 1001);
        let acl = Acl {
            rules: vec![AclRule {
                permission: Permission::Allow as i32,
                from_type: ActrType {
                    manufacturer: "acme".into(),
                    name: "client".into(),
                    version: "0.1.0".into(),
                },
                source_realm: Some(SourceRealm::AnyRealm(true)),
            }],
        };
        assert!(check_acl_permission(Some(&caller), &target, Some(&acl)).unwrap());
    }
}
