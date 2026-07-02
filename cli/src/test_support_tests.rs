use super::*;
use base64::Engine as _;

#[test]
fn local_actrix_config_includes_valid_renewal_token_secret() {
    let state_dir = TempDir::new().expect("temp state dir");
    let config_path = state_dir.path().join("actrix-e2e.toml");

    write_actrix_config(&config_path, state_dir.path(), 18_080, 13_478)
        .expect("config should be written");

    let content = fs::read_to_string(&config_path).expect("config should be readable");
    let secret_line = content
        .lines()
        .find(|line| line.trim_start().starts_with("renewal_token_secret = "))
        .expect("AIS renewal_token_secret should be configured");
    let secret = secret_line
        .split_once('=')
        .expect("secret assignment should contain '='")
        .1
        .trim()
        .trim_matches('"');
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(secret)
        .expect("renewal_token_secret should be valid base64");

    assert!(
        decoded.len() >= 32,
        "renewal_token_secret should decode to at least 32 bytes"
    );
}
