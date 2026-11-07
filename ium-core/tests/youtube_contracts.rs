//! The YouTube lane's contracts (#85) — written BEFORE any implementation
//! exists (the loop's RED step). At the pre-lane HEAD this file does not
//! even compile: `ium_core::google` and `ium_core::adapter_youtube` are
//! unresolved modules (E0432). Once the lane lands, every test here pins a
//! REAL wire contract or a privacy default — no network in this suite; the
//! reqwest call sites take their compile proof from CI (the #61 standing
//! note), while the protocol MATH is proven here.

use ium_core::adapter_youtube::{
    content_range, metadata_body, parse_resume, sanitize_title, status_range, YoutubeConfig, CHUNK,
    LANE, MAX_PARALLEL,
};
use ium_core::google;

const RFC7636_VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const RFC7636_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

#[test]
fn pkce_s256_matches_the_rfc7636_appendix_b_vector() {
    assert_eq!(google::pkce_challenge(RFC7636_VERIFIER), RFC7636_CHALLENGE);
}

#[test]
fn the_authorize_url_is_the_consent_screen_with_offline_access() {
    let (verifier, _) = google::pkce_pair();
    let url = google::authorize_url("cid.apps.googleusercontent.com", &verifier, "st4te");
    assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
    assert!(url.contains("client_id=cid.apps.googleusercontent.com"));
    assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A8737"));
    assert!(url.contains("response_type=code"));
    assert!(url.contains("scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fyoutube.upload"));
    assert!(url.contains("code_challenge_method=S256"));
    assert!(url.contains("state=st4te"));
    assert!(
        url.contains("access_type=offline"),
        "a refresh token must ride the grant"
    );
}

#[test]
fn a_fresh_pair_is_usable_and_never_repeats() {
    let (v1, c1) = google::pkce_pair();
    let (v2, c2) = google::pkce_pair();
    assert_eq!(google::pkce_challenge(&v1), c1);
    assert_ne!(v1, v2, "verifiers are random");
    assert_ne!(c1, c2);
    assert!(v1.len() >= 43, "RFC 7636 verifier floor");
}

#[test]
fn the_loopback_redirect_yields_code_and_state_percent_decoded() {
    let q = "/?code=4%2FxAVG&scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fyoutube.upload&state=st4te";
    let (code, state) = google::parse_redirect(q).expect("a code-bearing redirect parses");
    assert_eq!(code, "4/xAVG", "percent escapes decode");
    assert_eq!(state, "st4te");
}

#[test]
fn a_redirect_without_a_code_is_refused_and_mismatched_state_never_passes() {
    let q = "/?scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fyoutube.upload&state=st4te";
    assert!(
        google::parse_redirect(q).is_err(),
        "no code — the user said no"
    );
    let (_, state) = google::parse_redirect("/?code=abc&state=evil").unwrap();
    assert!(
        !google::state_matches(&state, "st4te"),
        "CSRF: the state must be OURS"
    );
    let (code, state) = google::parse_redirect("/?code=abc&state=st4te").unwrap();
    assert!(google::state_matches(&state, "st4te"));
    assert_eq!(code, "abc");
}

#[test]
fn tokens_round_trip_and_expiry_keeps_a_safety_skew() {
    let raw = r#"{"access_token":"a","expires_in":3600,"refresh_token":"r","token_type":"Bearer"}"#;
    let t = google::TokenSet::parse_at(raw, 0).expect("the token JSON parses");
    assert_eq!(t.access_token, "a");
    assert_eq!(t.refresh_token.as_deref(), Some("r"));
    assert!(!t.is_expired(0), "a fresh token is not expired at t=0");
    assert!(
        t.is_expired(3600 - 59),
        "the 60 s skew refreshes BEFORE the wire refuses"
    );
    let again = t.to_json();
    let re = google::TokenSet::parse_at(&again, 0).unwrap();
    assert_eq!(re.access_token, "a");
    assert_eq!(
        re.expires_at, t.expires_at,
        "the roundtrip keeps the expiry"
    );
}

#[test]
fn the_metadata_body_says_what_the_upload_means() {
    let body = metadata_body("My Video", "private", "22");
    assert!(body.contains("\"title\":\"My Video\""));
    assert!(
        body.contains("\"privacyStatus\":\"private\""),
        "private is the default, never public"
    );
    assert!(body.contains("\"categoryId\":\"22\""));
    assert!(body.contains("\"snippet\"") && body.contains("\"status\""));
}

#[test]
fn content_ranges_are_the_resumable_wire_language() {
    assert_eq!(
        content_range(0, 8_388_607, 20_000_000),
        "bytes 0-8388607/20000000"
    );
    assert_eq!(
        content_range(8_388_608, 19_999_999, 20_000_000),
        "bytes 8388608-19999999/20000000"
    );
    assert_eq!(
        status_range(20_000_000),
        "bytes */20000000",
        "the resume probe asks what the server already holds"
    );
}

#[test]
fn a_308_carries_the_received_prefix_and_the_final_body_names_the_video() {
    let r = parse_resume("308", Some("bytes=0-8388607"), "").expect("a 308 parses");
    assert_eq!(r.received_end, Some(8_388_607));
    assert!(!r.complete);
    assert!(r.video_id.is_none());

    let r = parse_resume("200", None, r#"{"id":"abc123","kind":"youtube#video"}"#)
        .expect("a 200 parses");
    assert!(r.complete);
    assert_eq!(r.video_id.as_deref(), Some("abc123"));
}

#[test]
fn titles_are_sanitized_and_capped_at_100_chars() {
    assert_eq!(sanitize_title("/home/u/My Clip.mp4"), "My Clip.mp4");
    assert_eq!(
        sanitize_title(""),
        "Upload",
        "an empty name never ships an empty title"
    );
    let long = sanitize_title(&"x".repeat(250));
    assert_eq!(long.chars().count(), 100, "YouTube's title cap");
}

#[test]
fn the_lane_declares_its_protocol_honestly() {
    assert_eq!(CHUNK, 8 * 1024 * 1024);
    assert_eq!(
        CHUNK % 262_144,
        0,
        "resumable chunks are multiples of 256 KiB"
    );
    assert_eq!(MAX_PARALLEL, 1, "one session is ONE sequential byte stream");
    assert_eq!(LANE, "youtube");
}

#[test]
fn missing_credentials_refuse_loudly_with_the_env_named() {
    let err = YoutubeConfig::required(None, None, None).expect_err("nothing configured");
    assert!(matches!(
        err,
        ium_core::adapter::AdapterError::AuthRequired(_)
    ));
    let err = YoutubeConfig::required(Some("cid"), None, None).expect_err("no secret");
    assert!(
        matches!(err, ium_core::adapter::AdapterError::AuthRequired(ref m) if m.contains("IUM_GOOGLE_CLIENT_SECRET"))
    );
    let err = YoutubeConfig::required(Some("cid"), Some("sec"), None).expect_err("no token");
    assert!(
        matches!(err, ium_core::adapter::AdapterError::AuthRequired(ref m) if m.contains("IUM_YOUTUBE_TOKEN"))
    );
}
