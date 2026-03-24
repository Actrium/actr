/// Validate a GitHub login as manufacturer name.
///
/// GitHub usernames: 1-39 chars, alphanumeric or hyphens, cannot start/end with hyphen,
/// no consecutive hyphens. We lower-case for storage.
pub fn validate_github_login(login: &str) -> Result<(), crate::MfrError> {
    let lower = login.to_ascii_lowercase();

    if lower.is_empty() || lower.len() > 39 {
        return Err(crate::MfrError::InvalidName(
            "name must be 1-39 characters".to_string(),
        ));
    }
    if lower.starts_with('-') || lower.ends_with('-') {
        return Err(crate::MfrError::InvalidName(
            "name must not start or end with a hyphen".to_string(),
        ));
    }
    if lower.contains("--") {
        return Err(crate::MfrError::InvalidName(
            "name must not contain consecutive hyphens".to_string(),
        ));
    }
    if !lower
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(crate::MfrError::InvalidName(
            "name must only contain alphanumeric characters or hyphens".to_string(),
        ));
    }
    Ok(())
}
