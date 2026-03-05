use actr_protocol::{ActrType, ActrTypeExt};
use std::cmp::Ordering;

/// Normalize an optional version:
/// - empty/blank string => None
/// - otherwise keep the original string
pub fn normalize_version(version: Option<String>) -> Option<String> {
    version.and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

/// Build stable type key using `manufacturer:name[:version]`.
pub fn type_key(actr_type: &ActrType) -> String {
    let mut normalized = actr_type.clone();
    normalized.version = normalize_version(normalized.version);
    normalized.to_string_repr()
}

/// Compare versions in descending order (latest first).
///
/// - String lexicographical order
/// - `None` is the lowest version
pub fn cmp_version_desc(a: Option<&str>, b: Option<&str>) -> Ordering {
    let left = a.and_then(|v| normalize_version(Some(v.to_string())));
    let right = b.and_then(|v| normalize_version(Some(v.to_string())));

    match (left.as_deref(), right.as_deref()) {
        (Some(av), Some(bv)) => bv.cmp(av),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_version() {
        assert_eq!(normalize_version(None), None);
        assert_eq!(normalize_version(Some(String::new())), None);
        assert_eq!(normalize_version(Some("   ".to_string())), None);
        assert_eq!(
            normalize_version(Some("  alpha  ".to_string())),
            Some("alpha".to_string())
        );
    }

    #[test]
    fn test_cmp_version_desc() {
        assert_eq!(cmp_version_desc(Some("2"), Some("1")), Ordering::Less);
        assert_eq!(cmp_version_desc(Some("alpha"), Some("2")), Ordering::Less);
        assert_eq!(cmp_version_desc(Some("1"), None), Ordering::Less);
        assert_eq!(cmp_version_desc(None, None), Ordering::Equal);
    }
}
