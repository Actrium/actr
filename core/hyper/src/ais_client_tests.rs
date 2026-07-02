use super::*;
use actr_protocol::ErrorResponse;
use reqwest::header::HeaderValue;

// ── client construction (no network) ────────────────────────────────────

#[test]
fn client_new_holds_endpoint_and_no_secret() {
    let c = AisClient::new("http://ais.example.com:8080");
    assert_eq!(c.endpoint, "http://ais.example.com:8080");
    assert!(c.realm_secret.is_none());

    let chained = c.with_realm_secret("s3cr3t");
    assert_eq!(chained.realm_secret.as_deref(), Some("s3cr3t"));
}

#[test]
fn with_realm_secret_is_chainable_idempotent_builder() {
    let c = AisClient::new("http://ais").with_realm_secret("abc");
    assert_eq!(c.realm_secret.as_deref(), Some("abc"));

    let c2 = c.with_realm_secret("def");
    assert_eq!(c2.endpoint, "http://ais");
    assert_eq!(c2.realm_secret.as_deref(), Some("def"));
}

// ── parse_retry_after ───────────────────────────────────────────────────

#[test]
fn parse_retry_after_valid_seconds() {
    let h = HeaderValue::from_static("120");
    assert_eq!(parse_retry_after(Some(&h)), Some(Duration::from_secs(120)));
}

#[test]
fn parse_retry_after_none() {
    assert_eq!(parse_retry_after(None), None);
}

#[test]
fn parse_retry_after_non_numeric_is_none() {
    // HTTP-date / non-numeric values are not supported → None.
    let h = HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT");
    assert_eq!(parse_retry_after(Some(&h)), None);

    let h2 = HeaderValue::from_static("abc");
    assert_eq!(parse_retry_after(Some(&h2)), None);
}

#[test]
fn parse_retry_after_zero() {
    let h = HeaderValue::from_static("0");
    assert_eq!(parse_retry_after(Some(&h)), Some(Duration::from_secs(0)));
}

// ── classify_renew_status ───────────────────────────────────────────────

#[test]
fn classify_status_client_errors() {
    assert!(matches!(
        classify_renew_status(400, None),
        RenewError::InvalidRequest(_)
    ));
    assert!(matches!(
        classify_renew_status(401, None),
        RenewError::TokenRejected
    ));
    assert!(matches!(
        classify_renew_status(403, None),
        RenewError::RealmUnavailable
    ));
}

#[test]
fn classify_status_rate_limited_carries_retry_after() {
    // With header present.
    assert!(matches!(
        classify_renew_status(429, Some(Duration::from_secs(30))),
        RenewError::RateLimited { retry_after: Some(d) } if d == Duration::from_secs(30)
    ));
    // Without header.
    assert!(matches!(
        classify_renew_status(429, None),
        RenewError::RateLimited { retry_after: None }
    ));
}

#[test]
fn classify_status_5xx_retryable() {
    for code in [500u16, 502, 503, 504] {
        assert!(
            matches!(classify_renew_status(code, None), RenewError::Retryable(_)),
            "{code} should be Retryable"
        );
    }
}

#[test]
fn classify_status_unknown_is_protocol() {
    assert!(matches!(
        classify_renew_status(418, None),
        RenewError::Protocol(_)
    ));
}

// ── classify_renew_error (delegates to status, retry_after=None) ─────────

#[test]
fn classify_error_maps_code_to_variant() {
    // 429 via ErrorResponse path never carries retry_after (header lost).
    let rate = ErrorResponse {
        code: 429,
        message: "slow down".into(),
    };
    assert!(matches!(
        classify_renew_error(&rate),
        RenewError::RateLimited { retry_after: None }
    ));

    let unauth = ErrorResponse {
        code: 401,
        message: "bad token".into(),
    };
    assert!(matches!(
        classify_renew_error(&unauth),
        RenewError::TokenRejected
    ));

    let boom = ErrorResponse {
        code: 503,
        message: "down".into(),
    };
    assert!(matches!(
        classify_renew_error(&boom),
        RenewError::Retryable(_)
    ));
}

#[test]
fn renew_error_display_messages() {
    assert!(format!("{}", RenewError::TokenRejected).contains("rejected"));
    assert!(format!("{}", RenewError::RealmUnavailable).contains("unavailable"));
    assert!(format!("{}", RenewError::InvalidRequest("bad".into())).contains("bad"));
}
