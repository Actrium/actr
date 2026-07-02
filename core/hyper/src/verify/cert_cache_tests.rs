use super::*;

#[tokio::test]
async fn cache_returns_cached_key_without_http() {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();
    let key_b64 = base64::engine::general_purpose::STANDARD.encode(verifying_key.to_bytes());

    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/mfr/test-mfr/verifying_key")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(format!(r#"{{"public_key":"{key_b64}"}}"#))
        .expect(1) // only called once, second time hits cache
        .create_async()
        .await;

    let cache = MfrCertCache::new(server.url());

    // first miss -> calls HTTP
    let k1 = cache.get_or_fetch("test-mfr", None).await.unwrap();
    // second hit -> no HTTP call
    let k2 = cache.get_or_fetch("test-mfr", None).await.unwrap();

    mock.assert_async().await;
    assert_eq!(k1.to_bytes(), k2.to_bytes());
    assert_eq!(k1.to_bytes(), verifying_key.to_bytes());
}

#[tokio::test]
async fn fetch_fails_on_404() {
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/mfr/unknown-mfr/verifying_key")
        .with_status(404)
        .create_async()
        .await;

    let cache = MfrCertCache::new(server.url());
    let result = cache.get_or_fetch("unknown-mfr", None).await;

    assert!(
        matches!(result, Err(HyperError::UntrustedManufacturer(_))),
        "404 should return UntrustedManufacturer, actual: {result:?}"
    );
}

#[tokio::test]
async fn debug_impl_contains_endpoint_and_ttl() {
    let cache = MfrCertCache::new("http://ais.example");
    let s = format!("{cache:?}");
    assert!(s.contains("MfrCertCache"));
    assert!(s.contains("ais_endpoint"));
    assert!(s.contains("ttl"));
    // finish_non_exhaustive() omits the cache map.
    assert!(!s.contains("cache"));
}

#[tokio::test]
async fn get_or_fetch_with_key_id_hits_cache_on_second_call() {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    let signing_key = SigningKey::generate(&mut OsRng);
    let key_b64 =
        base64::engine::general_purpose::STANDARD.encode(signing_key.verifying_key().to_bytes());

    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/mfr/m/verifying_key")
        .match_query(mockito::Matcher::Any)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(format!(r#"{{"public_key":"{key_b64}"}}"#))
        .expect(1) // second call must hit the composite-key cache
        .create_async()
        .await;

    let cache = MfrCertCache::new(server.url());
    let k1 = cache.get_or_fetch("m", Some("v1")).await.unwrap();
    let k2 = cache.get_or_fetch("m", Some("v1")).await.unwrap();
    mock.assert_async().await;
    assert_eq!(k1.to_bytes(), k2.to_bytes());
}

#[tokio::test]
async fn fetch_fails_on_malformed_json_body() {
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/mfr/badjson/verifying_key")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body("this is not json")
        .create_async()
        .await;

    let cache = MfrCertCache::new(server.url());
    let err = cache.get_or_fetch("badjson", None).await.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("parse"),
        "malformed JSON should surface a parse error, got: {msg}"
    );
}

#[tokio::test]
async fn fetch_fails_on_invalid_base64_key() {
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/mfr/badb64/verifying_key")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"public_key":"@@@not-valid-base64@@@"}"#)
        .create_async()
        .await;

    let cache = MfrCertCache::new(server.url());
    let err = cache.get_or_fetch("badb64", None).await.unwrap_err();
    assert!(
        err.to_string().contains("base64"),
        "invalid base64 should be reported, got: {err}"
    );
}

#[tokio::test]
async fn fetch_fails_on_wrong_key_length() {
    // Valid base64, but decodes to 16 bytes (not 32).
    let short_b64 = base64::engine::general_purpose::STANDARD.encode([0u8; 16]);
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/mfr/badlen/verifying_key")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(format!(r#"{{"public_key":"{short_b64}"}}"#))
        .create_async()
        .await;

    let cache = MfrCertCache::new(server.url());
    let err = cache.get_or_fetch("badlen", None).await.unwrap_err();
    assert!(
        err.to_string().contains("32 bytes"),
        "wrong length should be reported, got: {err}"
    );
}
