//! Shared input validators used across the crates.io and docs.rs
//! layers. Both consume the same `[A-Za-z0-9_-]+` rule that crates.io
//! itself enforces on publish, so housing the check here keeps the two
//! sides from drifting.

/// Upper bound on crate-name length we'll accept before issuing any
/// upstream call. crates.io's own limit is 64 chars; matching it here
/// keeps the validator honest.
pub(crate) const MAX_CRATE_NAME_LEN: usize = 64;

/// Validate a crate name against crates.io's published character set
/// and length. Returns a plain-string error message so each caller can
/// wrap it in their own use-case error variant.
///
/// The check is deliberately strict: rejecting anything outside
/// `[A-Za-z0-9_-]+` before the network ensures we can't smuggle
/// path-traversal segments or query strings into the upstream URL.
pub(crate) fn validate_crate_name_chars(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("crate name must not be empty".into());
    }
    if name.len() > MAX_CRATE_NAME_LEN {
        return Err(format!(
            "crate name longer than {MAX_CRATE_NAME_LEN} characters"
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!(
            "crate name contains disallowed characters: {name:?}"
        ));
    }
    Ok(())
}
