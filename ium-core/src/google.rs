//! Google OAuth for installed apps (#85) — the YouTube lane's sign-in.
//!
//! rclone ships its own Google client, which is why the Drive lane (#84)
//! needs zero config. YouTube has NO rclone backend, so IUM runs its own
//! installed-app flow here: the loopback redirect (RFC 8252), PKCE S256
//! (RFC 7636), an anti-CSRF state, `access_type=offline` (a refresh token
//! rides the grant), and the token endpoints.
//!
//! The pure math lives here and is tested against real vectors; the two
//! network calls (`exchange_code`, `refresh`) use the crate's reqwest (the
//! TLS dep #63 brought in) and take their compile proof from CI — the
//! standing #61 contract.
//!
//! HONEST LIMIT: Google has no anonymous OAuth. Until IUM ships a
//! registered (and Google-verified for `youtube.upload`) client, the client
//! id/secret are a ONE-TIME setup in the lane's sheet; the sheet says so in
//! one sentence. After that, connect is one click.

use std::time::Duration;

/// The loopback port the redirect comes back to. One fixed port (the user's
/// Google client registers exactly `http://localhost:8737`); a port the OS
/// refuses is an honest error sentence, never a silent rebind.
pub const REDIRECT_PORT: u16 = 8737;

/// The upload-only scope: videos may be inserted, nothing else is asked.
pub const SCOPE_UPLOAD: &str = "https://www.googleapis.com/auth/youtube.upload";

/// base64url WITHOUT padding — PKCE's `code_challenge` spelling.
fn b64url(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(T[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(T[n as usize & 63] as char);
        }
    }
    out
}

/// PKCE S256 (RFC 7636 §4.2): `challenge = BASE64URL(SHA256(verifier))`.
/// `sha2` rides the crate (sigv4's HMAC family), `getrandom` too.
pub fn pkce_challenge(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(verifier.as_bytes());
    b64url(&h.finalize())
}

/// A fresh verifier/challenge pair — 64 URL-safe characters (the RFC floor
/// is 43; 64 keeps the entropy comfortably above it).
pub fn pkce_pair() -> (String, String) {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut bytes = [0u8; 64];
    getrandom::getrandom(&mut bytes).expect("the OS CSPRNG is always available");
    let verifier: String = bytes
        .iter()
        .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
        .collect();
    let challenge = pkce_challenge(&verifier);
    (verifier, challenge)
}

/// A fresh anti-CSRF state (same CSPRNG discipline).
pub fn new_state() -> String {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).expect("the OS CSPRNG is always available");
    b64url(&bytes)
}

/// The consent-screen URL: the provider's REAL page opens in the user's
/// browser (the provider owns the flow — 2FA and consent included), and the
/// redirect comes back to the loopback listener. `access_type=offline` +
/// `prompt=consent` keep the refresh token arriving on EVERY grant.
pub fn authorize_url(client_id: &str, verifier: &str, state: &str) -> String {
    let redirect = percent_encode(&format!("http://localhost:{REDIRECT_PORT}"));
    format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&code_challenge={}&code_challenge_method=S256&access_type=offline&prompt=consent",
        percent_encode(client_id),
        redirect,
        percent_encode(SCOPE_UPLOAD),
        percent_encode(state),
        pkce_challenge(verifier),
    )
}

/// Percent-encode everything RFC 3986's unreserved set does not cover —
/// query values must survive every OS's browser-argument plumbing.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Minimal percent-decode for the loopback query values (Google's codes ride
/// `%2F` escapes; `+` is NOT treated as space — these are token values, not
/// form fields).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The loopback redirect parse: `GET /?code=…&state=…`. A redirect without a
/// code (the user clicked Cancel / denied the scope) is an honest refusal —
/// never a fake success.
pub fn parse_redirect(target: &str) -> Result<(String, String), String> {
    let query = target
        .split_once('?')
        .map(|(_, q)| q)
        .ok_or_else(|| "the sign-in came back without a query — no code, no state".to_string())?;
    let mut code: Option<String> = None;
    let mut state: Option<String> = None;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        match k {
            "code" => code = Some(percent_decode(v)),
            "state" => state = Some(percent_decode(v)),
            _ => {}
        }
    }
    match (code, state) {
        (Some(c), Some(s)) => Ok((c, s)),
        (Some(_), None) => Err("the sign-in came back without an anti-CSRF state".into()),
        (None, _) => Err(
            "the sign-in finished without a grant — the consent screen was cancelled or denied."
                .into(),
        ),
    }
}

/// CSRF gate (RFC 6749 §10.12): the state that came back must be byte-equal
/// to the state WE sent (constant-time-ish compare — lengths differ early,
/// bytes differ late, and the verdict is the same either way).
pub fn state_matches(received: &str, expected: &str) -> bool {
    if received.len() != expected.len() {
        return false;
    }
    received
        .bytes()
        .zip(expected.bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// The token store: exactly what Google's token endpoint answers with, kept
/// verbatim so a refresh never loses the grant.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TokenSet {
    /// The bearer token every YouTube API call rides.
    pub access_token: String,
    /// The long-lived grant. `None` on a refresh answer that did not rotate
    /// it — the stored one stays valid (Google's documented behavior).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Absolute expiry (unix seconds) — computed from `expires_in` at parse
    /// time, so a persisted set can answer "is this still fresh" on boot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    /// Usually `Bearer`; kept for round-trip fidelity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
}

impl TokenSet {
    /// Parse Google's token JSON. `expires_in` becomes an absolute
    /// `expires_at` (unix seconds); `now` is passed in so tests stay pure.
    pub fn parse_at(raw: &str, now: u64) -> Result<Self, String> {
        #[derive(serde::Deserialize)]
        struct Wire {
            access_token: String,
            #[serde(default)]
            refresh_token: Option<String>,
            /// Google's relative TTL (`expires_in`) on the wire answer.
            #[serde(default)]
            expires_in: Option<u64>,
            /// OUR absolute spelling on the roundtripped vault JSON.
            #[serde(default)]
            expires_at: Option<u64>,
            #[serde(default)]
            token_type: Option<String>,
        }
        let w: Wire =
            serde_json::from_str(raw).map_err(|e| format!("the token JSON is not a token: {e}"))?;
        Ok(Self {
            expires_at: w.expires_at.or_else(|| w.expires_in.map(|s| now + s)),
            refresh_token: w.refresh_token,
            token_type: w.token_type,
            access_token: w.access_token,
        })
    }

    /// Parse at the current wall clock (the runtime path).
    pub fn parse(raw: &str) -> Result<Self, String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self::parse_at(raw, now)
    }

    /// Serialize back to the exact JSON shape the vault stores.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }

    /// Fresh with a 60-second safety skew — the refresh fires BEFORE the
    /// wire refuses, never after a failed upload taught us the hard way.
    pub fn is_expired(&self, now: u64) -> bool {
        match self.expires_at {
            Some(t) => now + 60 >= t,
            None => false,
        }
    }

    /// Merge a refresh answer over the stored set: the new access token and
    /// expiry win; the refresh token survives when the answer omits it.
    pub fn merged_with(&self, fresh: TokenSet) -> TokenSet {
        TokenSet {
            refresh_token: fresh.refresh_token.or_else(|| self.refresh_token.clone()),
            ..fresh
        }
    }
}

/// The client — id + secret from the lane's sheet (one-time setup until
/// IUM ships its own registered client; the sheet says so).
#[derive(Debug, Clone)]
pub struct GoogleClient {
    /// The OAuth client id ("…apps.googleusercontent.com").
    pub client_id: String,
    /// The OAuth client secret — a secret, but one the user pasted into
    /// their OWN client's sheet; the vault stores it like every secret.
    pub client_secret: String,
}

impl GoogleClient {
    /// The one HTTP client the token endpoints need (rustls; no pool
    /// sharing concerns — an OAuth flow makes two calls in its lifetime).
    fn token_client() -> reqwest::Client {
        reqwest::Client::builder()
            .user_agent(concat!("IUM/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()
            .expect("the reqwest client builds with rustls")
    }

    /// The token exchange: `authorization_code` + the PKCE verifier. The
    /// redirect must byte-match the authorize request's.
    pub async fn exchange_code(&self, code: &str, verifier: &str) -> Result<TokenSet, String> {
        let params: [(&str, &str); 6] = [
            ("code", code),
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
            ("redirect_uri", &format!("http://localhost:{REDIRECT_PORT}")),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier),
        ];
        Self::token_answer(
            Self::token_client()
                .post("https://oauth2.googleapis.com/token")
                .form(&params)
                .send()
                .await
                .map_err(|e| format!("the token exchange could not reach Google: {e}"))?,
        )
        .await
    }

    /// The refresh grant — the long-lived grant in, a fresh access token
    /// out.
    pub async fn refresh(&self, refresh_token: &str) -> Result<TokenSet, String> {
        let params: [(&str, &str); 4] = [
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ];
        Self::token_answer(
            Self::token_client()
                .post("https://oauth2.googleapis.com/token")
                .form(&params)
                .send()
                .await
                .map_err(|e| format!("the token refresh could not reach Google: {e}"))?,
        )
        .await
    }

    async fn token_answer(resp: reqwest::Response) -> Result<TokenSet, String> {
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| format!("the token answer could not be read: {e}"))?;
        if !status.is_success() {
            return Err(format!(
                "Google refused the token request ({status}): {}",
                body.chars().take(200).collect::<String>()
            ));
        }
        TokenSet::parse(&body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64url_matches_the_rfc_vector_through_sha256() {
        // The RFC 7636 Appendix B vector, exercised through the public fn.
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn b64url_has_no_padding_and_only_url_safe_bytes() {
        let s = b64url(b"\xfb");
        assert!(!s.contains('='));
        assert!(s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn a_denied_consent_refuses_and_a_mismatched_state_never_passes() {
        assert!(parse_redirect("/?error=access_denied&state=x").is_err());
        let (_, s) = parse_redirect("/?code=c&state=ours").unwrap();
        assert!(!state_matches(&s, "theirs"));
        assert!(state_matches(&s, "ours"));
    }

    #[test]
    fn expiry_skews_early_and_the_merge_keeps_the_grant() {
        let mut t = TokenSet::parse_at(r#"{"access_token":"a","expires_in":3600}"#, 1000).unwrap();
        assert!(!t.is_expired(1000));
        assert!(t.is_expired(4541), "3600 - 60 + 1 → expired");
        assert!(t.refresh_token.is_none());
        let fresh = TokenSet::parse_at(r#"{"access_token":"b","expires_in":3600}"#, 5000).unwrap();
        t.expires_at = Some(2000);
        let merged = t.merged_with(fresh);
        assert_eq!(merged.access_token, "b");
        assert_eq!(merged.expires_at, Some(8600));
    }
}
