//! ACL (Access Control List) 权限检查
//!
//! 从入站消息的 caller_id 和目标 actor_id 出发，依据配置的 ACL 规则
//! 判定是否允许本次调用。本模块是纯函数，无任何 IO 依赖，适用于
//! native 和 wasm32 两种运行目标。

use actr_protocol::{Acl, AclRule, ActrId, ActrIdExt as _};

/// 检查调用方是否有权限访问目标 Actor
///
/// # 返回值
/// - `Ok(true)`: 允许
/// - `Ok(false)`: 拒绝
/// - `Err(String)`: 检查过程异常（应视为拒绝）
///
/// # 评估逻辑
/// 1. 无 caller_id（本地调用）——始终放行
/// 2. 未配置 ACL——默认放行（兼容旧版配置）
/// 3. ACL 已配置但规则列表为空——全部拒绝（安全默认）
/// 4. Deny-first: 任何命中 DENY 的规则立即拒绝
/// 5. 存在至少一条 ALLOW 命中——放行
/// 6. 无规则命中——拒绝
pub fn check_acl_permission(
    caller_id: Option<&ActrId>,
    target_id: &ActrId,
    acl: Option<&Acl>,
) -> Result<bool, String> {
    // 1. 本地调用始终放行
    if caller_id.is_none() {
        tracing::trace!("ACL: local call, allowing");
        return Ok(true);
    }

    let caller = caller_id.unwrap();

    // 2. 未配置 ACL——默认放行
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

    // 3. 规则列表为空——全部拒绝
    if acl.rules.is_empty() {
        tracing::warn!(
            "ACL: empty rule set, denying {} -> {} (default deny)",
            caller.to_string_repr(),
            target_id.to_string_repr(),
        );
        return Ok(false);
    }

    // 4 & 5. Deny-first 评估
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

    // 6. 无规则命中——拒绝
    tracing::warn!(
        "ACL: no matching rule, denying {} -> {} (default deny)",
        caller.to_string_repr(),
        target_id.to_string_repr(),
    );
    Ok(false)
}

/// 判断单条 ACL 规则是否匹配给定 caller
fn matches_rule(caller: &ActrId, rule: &AclRule) -> bool {
    use actr_protocol::acl_rule::SourceRealm;

    // 类型精确匹配（manufacturer + name + version）
    if caller.r#type.manufacturer != rule.from_type.manufacturer
        || caller.r#type.name != rule.from_type.name
        || caller.r#type.version != rule.from_type.version
    {
        return false;
    }

    // Realm 匹配
    match &rule.source_realm {
        None | Some(SourceRealm::AnyRealm(_)) => true,
        Some(SourceRealm::RealmId(id)) => caller.realm.realm_id == *id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actr_protocol::{ActrType, Realm, acl_rule::Permission};

    fn make_id(manufacturer: &str, name: &str, realm_id: u32) -> ActrId {
        ActrId {
            serial_number: 0xaabb,
            r#type: ActrType {
                manufacturer: manufacturer.into(),
                name: name.into(),
                version: String::new(),
            },
            realm: Realm { realm_id },
        }
    }

    fn make_rule(manufacturer: &str, name: &str, perm: Permission) -> AclRule {
        AclRule {
            permission: perm as i32,
            from_type: ActrType {
                manufacturer: manufacturer.into(),
                name: name.into(),
                version: String::new(),
            },
            source_realm: None,
        }
    }

    #[test]
    fn local_call_always_allowed() {
        let target = make_id("acme", "svc", 1);
        assert!(check_acl_permission(None, &target, None).unwrap());
    }

    #[test]
    fn no_acl_allows_by_default() {
        let caller = make_id("acme", "client", 1);
        let target = make_id("acme", "svc", 1);
        assert!(check_acl_permission(Some(&caller), &target, None).unwrap());
    }

    #[test]
    fn empty_rules_denies() {
        let caller = make_id("acme", "client", 1);
        let target = make_id("acme", "svc", 1);
        let acl = Acl { rules: vec![] };
        assert!(!check_acl_permission(Some(&caller), &target, Some(&acl)).unwrap());
    }

    #[test]
    fn deny_overrides_allow() {
        let caller = make_id("acme", "client", 1);
        let target = make_id("acme", "svc", 1);
        let acl = Acl {
            rules: vec![
                make_rule("acme", "client", Permission::Allow),
                make_rule("acme", "client", Permission::Deny),
            ],
        };
        assert!(!check_acl_permission(Some(&caller), &target, Some(&acl)).unwrap());
    }

    #[test]
    fn allow_when_matched() {
        let caller = make_id("acme", "client", 1);
        let target = make_id("acme", "svc", 1);
        let acl = Acl {
            rules: vec![make_rule("acme", "client", Permission::Allow)],
        };
        assert!(check_acl_permission(Some(&caller), &target, Some(&acl)).unwrap());
    }

    #[test]
    fn no_match_denies() {
        let caller = make_id("acme", "client", 1);
        let target = make_id("acme", "svc", 1);
        let acl = Acl {
            rules: vec![make_rule("other", "other", Permission::Allow)],
        };
        assert!(!check_acl_permission(Some(&caller), &target, Some(&acl)).unwrap());
    }
}
