//! An auth node's signing key, JWKS and tokens (spec X13).
//!
//! RS256 is hand-rolled over the `rsa` crate: the JWS compact form is
//! `base64url(header) . base64url(claims) . base64url(PKCS#1 v1.5 SHA-256 signature)`,
//! no padding. Key IO lives in the `kaanal` binary (X5); this module only parses, generates
//! and signs. RUSTSEC-2023-0071 (an RSA timing side channel) does not matter for a local mock
//! that signs synthetic tokens (R1.16). Nothing here ever prints a password or the key.
//!
//! `POST /login` bodies are read by [`read_login`] before the chain runs; [`respond`] builds
//! the answer after it succeeded.

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::body::{ReadError, read_capped};
use axum::extract::Request;
use axum::http::header;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use indexmap::IndexMap;
use kaanal_core::config::{NodeConfig, UserConfig};
use kaanal_core::diagnostics::Located;
use kaanal_core::{Config, Responder};
use rsa::PrivateKey as RsaPrivateKey;
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey, LineEnding};
use rsa::signature::{SignatureEncoding, Signer};
use rsa::traits::PublicKeyParts;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

/// **Private.** Token lifetime when `token_ttl` is absent (R1.16).
const DEFAULT_TTL: Duration = Duration::from_secs(3600);

/// An RS256 signing key with its RFC 7638 `kid` and public JWK. `Debug` shows only the `kid`.
#[derive(Clone)]
pub struct AuthKey {
    /// **Private.** The RSA private key.
    private: RsaPrivateKey,
    /// **Private.** RFC 7638 thumbprint of the public key.
    kid: String,
    /// **Private.** The public key as a JWK (`kty`, `use`, `alg`, `kid`, `n`, `e`).
    jwk: Value,
}

impl fmt::Debug for AuthKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthKey")
            .field("kid", &self.kid)
            .finish_non_exhaustive()
    }
}

impl AuthKey {
    /// A new 2048-bit key from OS entropy.
    ///
    /// # Errors
    /// The OS random source failed.
    pub fn generate() -> Result<AuthKey, String> {
        RsaPrivateKey::new(&mut getrandom::SysRng, 2048)
            .map(AuthKey::from_private)
            .map_err(|e| format!("cannot generate a signing key: {e}"))
    }

    /// Parses a PKCS#8 PEM private key (what [`AuthKey::to_pem`] writes).
    ///
    /// # Errors
    /// The text is not a PKCS#8 RSA private key (the message never contains the key).
    pub fn from_pem(pem: &str) -> Result<AuthKey, String> {
        RsaPrivateKey::from_pkcs8_pem(pem)
            .map(AuthKey::from_private)
            // The crate's own text is dropped: it can name the wrong thing (an EC key reports
            // RSA's OID) and may quote parts of the file.
            .map_err(|_| {
                "not a PKCS#8 RSA private key (expected an unencrypted RSA key in PEM form, \
                 starting with -----BEGIN PRIVATE KEY-----)"
                    .to_string()
            })
    }

    /// The key as PKCS#8 PEM with LF line endings, for the key file.
    ///
    /// # Errors
    /// Encoding failed (not expected for a key this type built).
    pub fn to_pem(&self) -> Result<String, String> {
        self.private
            .to_pkcs8_pem(LineEnding::LF)
            .map(|p| p.to_string())
            .map_err(|e| format!("cannot encode the signing key: {e}"))
    }

    /// The key id: the RFC 7638 thumbprint of the public key.
    pub fn kid(&self) -> &str {
        &self.kid
    }

    /// The JWKS document `GET /.well-known/jwks.json` serves: `{"keys":[<this key>]}`.
    pub fn jwks(&self) -> Value {
        json!({ "keys": [self.jwk] })
    }

    /// **Private.** Derives the `kid` and the public JWK once.
    fn from_private(private: RsaPrivateKey) -> AuthKey {
        // minimal big-endian bytes, as a JWK writes `n` and `e` (RFC 7518 6.3.1)
        let (n, e) = (
            URL_SAFE_NO_PAD.encode(private.n_bytes()),
            URL_SAFE_NO_PAD.encode(private.e_bytes()),
        );
        let kid = thumbprint(&n, &e);
        let jwk = json!({ "kty": "RSA", "use": "sig", "alg": "RS256", "kid": kid, "n": n, "e": e });
        AuthKey { private, kid, jwk }
    }

    /// **Private.** The signed compact token for `claims`.
    fn sign(&self, claims: &Value) -> String {
        let header = json!({ "alg": "RS256", "typ": "JWT", "kid": self.kid });
        let input = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(header.to_string()),
            URL_SAFE_NO_PAD.encode(claims.to_string())
        );
        let sig = SigningKey::<Sha256>::new(self.private.clone()).sign(input.as_bytes());
        format!("{input}.{}", URL_SAFE_NO_PAD.encode(sig.to_bytes()))
    }

    /// **Private.** The length of [`AuthKey::sign`]'s token for `claims`, without signing: the header
    /// and the claims are measured as they are written, and a PKCS#1 v1.5 signature is always as long
    /// as the modulus (RFC 8017 8.2.1), so the three base64url parts and two dots add up exactly.
    fn sign_len(&self, claims: &Value) -> usize {
        let header = json!({ "alg": "RS256", "typ": "JWT", "kid": self.kid }).to_string();
        b64_len(header.len())
            + 1
            + b64_len(claims.to_string().len())
            + 1
            + b64_len(self.private.size())
    }
}

/// **Private.** Unpadded base64url length of `n` bytes: `ceil(4n / 3)`.
const fn b64_len(n: usize) -> usize {
    (4 * n).div_ceil(3)
}

/// RFC 7638 JWK thumbprint of an RSA public key given as base64url `n` and `e`: the SHA-256
/// of `{"e":…,"kty":"RSA","n":…}` (members sorted, no whitespace), base64url.
pub(crate) fn thumbprint(n: &str, e: &str) -> String {
    let canonical = format!(r#"{{"e":"{e}","kty":"RSA","n":"{n}"}}"#);
    URL_SAFE_NO_PAD.encode(Sha256::digest(canonical.as_bytes()))
}

/// What `POST /login` needs: the key and the auth node's accounts, claims and token lifetime.
#[derive(Debug)]
pub(crate) struct Issuer {
    /// The signing key (also serves the JWKS).
    pub(crate) key: AuthKey,
    /// **Private.** The accounts (`Debug` redacts passwords).
    users: Vec<UserConfig>,
    /// **Private.** The node's extra claims; a user's own win.
    claims: IndexMap<String, Value>,
    /// **Private.** Token lifetime.
    ttl: Duration,
    /// **Private.** `iss`: `http://localhost:<bound port>`, never the `Host` header (X18).
    iss: String,
}

impl Issuer {
    /// The issuer of `cfg`'s auth node, or `None` when it has none.
    ///
    /// # Errors
    /// The config has an auth node and `key` is `None`: the server fails closed (X5).
    pub(crate) fn from_config(
        cfg: &Config,
        key: Option<AuthKey>,
        port: u16,
    ) -> Result<Option<Issuer>, Located> {
        let Some((name, a)) = cfg.nodes.iter().find_map(|(n, c)| match c {
            NodeConfig::Auth(a) => Some((n, a)),
            _ => None,
        }) else {
            return Ok(None);
        };
        let key = key.ok_or_else(|| {
            Located::new(
                Vec::new(),
                format!("auth node '{name}' needs a signing key (kaanal up provides one)"),
            )
        })?;
        Ok(Some(Issuer {
            key,
            users: a.users.clone(),
            claims: a.claims.clone(),
            ttl: a.token_ttl.unwrap_or(DEFAULT_TTL),
            iss: format!("http://localhost:{port}"),
        }))
    }

    /// The `200` body for good credentials, `None` (a 401) for an unknown user or a wrong
    /// password alike (X8). Plain comparison: synthetic accounts on a local mock.
    pub(crate) fn login(&self, username: &str, password: &str) -> Option<Value> {
        let user = self
            .users
            .iter()
            .find(|u| u.username == username && u.password == password)?;
        Some(self.reply(user, &self.token(user, unix_now(), rand::random())))
    }

    /// The size in bytes of the `200` body [`Issuer::login`] would build for these credentials, or of
    /// the `401` body for an unknown user or a wrong password, with no signing: a login's token has a
    /// length that depends on the account, which is all the mock needs to charge the link and `per_kb`.
    pub(crate) fn login_len(&self, username: &str, password: &str) -> u64 {
        let Some(user) = self
            .users
            .iter()
            .find(|u| u.username == username && u.password == password)
        else {
            return json_len(&invalid_credentials());
        };
        // jti is 32 hex digits whatever its value, and now has its present digit count
        let claims = self.claims(user, unix_now(), 0);
        let token = "x".repeat(self.key.sign_len(&claims));
        json_len(&self.reply(user, &token))
    }

    /// **Private.** The `200` body around a token: what [`Issuer::login`] answers.
    fn reply(&self, user: &UserConfig, token: &str) -> Value {
        let mut body = json!({
            "access_token": token,
            "token_type": "Bearer",
            "expires_in": self.ttl.as_secs(),
        });
        if !user.scopes.is_empty() {
            body["scope"] = Value::String(user.scopes.join(" "));
        }
        body
    }

    /// **Private.** A signed token for `user` issued at `now` (Unix seconds) with id `jti`.
    fn token(&self, user: &UserConfig, now: u64, jti: u128) -> String {
        self.key.sign(&self.claims(user, now, jti))
    }

    /// **Private.** The claims of a token for `user`: the node's claims, then the user's (which win),
    /// then sub, iss, iat, exp, jti, roles and scope (only with scopes). The standard claims go last,
    /// so they win even if a `Config` built without `from_yaml` carries a reserved name (rejected at
    /// load, X3).
    fn claims(&self, user: &UserConfig, now: u64, jti: u128) -> Value {
        let mut c = Map::new();
        for (k, v) in self.claims.iter().chain(&user.claims) {
            c.insert(k.clone(), v.clone());
        }
        c.insert("sub".into(), user.username.clone().into());
        c.insert("iss".into(), self.iss.clone().into());
        c.insert("iat".into(), now.into());
        // ttl is at most 1 day (config), so this never saturates in practice.
        c.insert("exp".into(), now.saturating_add(self.ttl.as_secs()).into());
        c.insert("jti".into(), format!("{jti:032x}").into());
        c.insert("roles".into(), user.roles.clone().into());
        if !user.scopes.is_empty() {
            c.insert("scope".into(), user.scopes.join(" ").into());
        }
        Value::Object(c)
    }
}

/// **Private.** Seconds since the Unix epoch (0 if the clock is before it).
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// **Private.** The serialized length of `v`, as the mock writes it.
fn json_len(v: &Value) -> u64 {
    v.to_string().len() as u64
}

/// **Private.** The `401` body of a login (X8): an unknown user and a wrong password alike.
fn invalid_credentials() -> Value {
    json!({ "error": "invalid_credentials" })
}

/// **Private.** Largest login body Kaanal reads (spec section 5).
const MAX_LOGIN_BODY: usize = 16 * 1024;

/// The two fields `POST /login` reads. Extra fields (`grant_type`, …) are ignored; serde's
/// derive rejects duplicate, missing and non-string ones (P12). `Debug` hides the password. A JSON
/// body must be an object: serde would also read a struct from an array, which [`read_login`] refuses.
#[derive(Deserialize)]
pub(crate) struct Credentials {
    /// **Private.** The account name.
    username: String,
    /// **Private.** The password as sent.
    password: String,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// A login request rejected before the credentials are checked: the status and the `error`
/// code of the body (P10). Never a parser message: those can quote the password.
pub(crate) type Reject = (u16, &'static str);

/// Reads a `POST /login` body before the chain runs (spec section 5). `Content-Type` (case-insensitive,
/// parameters ignored) must be `application/json` or `application/x-www-form-urlencoded`, else
/// 415 (decided from the header alone); a body over 16 KiB is 413; anything serde rejects is 400.
pub(crate) async fn read_login(req: Request) -> Result<Credentials, Reject> {
    let kind = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
        });
    let json = match kind.as_deref() {
        Some("application/json") => true,
        Some("application/x-www-form-urlencoded") => false,
        // `fetch` with `JSON.stringify` and no header sends text/plain (documented in auth.md).
        _ => return Err((415, "unsupported_media_type")),
    };
    let body = read_capped(req, MAX_LOGIN_BODY)
        .await
        .map_err(|e| match e {
            ReadError::TooLarge => (413, "request_too_large"),
            ReadError::Broken => (400, "invalid_request"),
        })?;
    let creds = if json {
        // an array (`["alice","pw"]`) is not a login, though serde would read it into the struct
        let object = body.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{');
        object
            .then(|| serde_json::from_slice::<Credentials>(&body).ok())
            .flatten()
    } else {
        serde_urlencoded::from_bytes::<Credentials>(&body).ok()
    };
    creds.ok_or((400, "invalid_request"))
}

/// The status and body of a built-in route whose chain succeeded. `login` is what
/// [`read_login`] returned; it is `Some` whenever `r` is [`Responder::Login`].
pub(crate) fn respond(
    r: Responder,
    issuer: Option<&Issuer>,
    login: Option<Result<Credentials, Reject>>,
) -> (u16, Value) {
    // `serve` refuses to start an auth config without a key, so this is only defensive.
    let Some(issuer) = issuer else {
        return (500, json!({ "error": "no signing key" }));
    };
    match (r, login) {
        (Responder::Jwks, _) => (200, issuer.key.jwks()),
        (Responder::Login, Some(Ok(c))) => match issuer.login(&c.username, &c.password) {
            Some(body) => (200, body),
            // X8: wrong password and unknown user alike, after the full chain latency.
            None => (401, invalid_credentials()),
        },
        (Responder::Login, Some(Err((status, code)))) => (status, json!({ "error": code })),
        (Responder::Login, None) => (500, json!({ "error": "login body not read" })),
    }
}

/// The size in bytes of the body [`respond`] gives a login, without signing the token (see
/// [`Issuer::login_len`]). `login` is what [`read_login`] returned; the arms mirror [`respond`], and
/// a test holds the two together.
pub(crate) fn login_reply_len(issuer: Option<&Issuer>, login: &Result<Credentials, Reject>) -> u64 {
    match (issuer, login) {
        (None, _) => json_len(&json!({ "error": "no signing key" })),
        (Some(_), Err((_, code))) => json_len(&json!({ "error": code })),
        (Some(i), Ok(c)) => i.login_len(&c.username, &c.password),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::sync::OnceLock;

    /// One key per test binary: generation is the slow part (spec section 7).
    fn key() -> &'static AuthKey {
        static KEY: OnceLock<AuthKey> = OnceLock::new();
        KEY.get_or_init(|| AuthKey::generate().unwrap())
    }

    const CFG: &str = "version: 1
nodes:
  login:
    type: auth
    token_ttl: 2h
    claims: { aud: my-app, tier: free }
    users:
      - { username: alice, password: secret, roles: [admin], scopes: [read, write], claims: { tier: pro, email: a@x.dev } }
      - { username: bob, password: pw }
";

    fn issuer() -> Issuer {
        let cfg = Config::from_yaml(CFG).unwrap();
        Issuer::from_config(&cfg, Some(key().clone()), 8080)
            .unwrap()
            .unwrap()
    }

    /// A token's claims, decoded without checking the signature.
    fn claims_of(token: &str) -> Value {
        let payload = token.split('.').nth(1).unwrap();
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap()
    }

    fn verifier(iss: &str) -> jsonwebtoken::Validation {
        let mut v = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
        v.set_issuer(&[iss]);
        // Nothing enforces `aud` (X10); node claims may carry one.
        v.validate_aud = false;
        v
    }

    #[test]
    fn thumbprint_matches_rfc_7638() {
        // RFC 7638 section 3.1: the example RSA key and its thumbprint.
        let n = "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw";
        assert_eq!(
            thumbprint(n, "AQAB"),
            "NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs"
        );
    }

    #[test]
    fn jwks_has_one_rs256_signing_key_named_by_its_thumbprint() {
        let jwks = key().jwks();
        let keys = jwks["keys"].as_array().unwrap();
        assert_eq!(keys.len(), 1, "{jwks}");
        let k = &keys[0];
        assert_eq!(
            [&k["kty"], &k["use"], &k["alg"], &k["e"]],
            [
                &json!("RSA"),
                &json!("sig"),
                &json!("RS256"),
                &json!("AQAB")
            ]
        );
        let n = k["n"].as_str().unwrap();
        assert_eq!(k["kid"], json!(key().kid()));
        assert_eq!(key().kid(), thumbprint(n, "AQAB"));
        assert_eq!(
            URL_SAFE_NO_PAD.decode(n).unwrap().len(),
            256,
            "2048-bit modulus"
        );
        assert_eq!(k.as_object().unwrap().len(), 6, "{k}");
    }

    #[test]
    fn pem_round_trips_and_debug_hides_the_key() {
        let pem = key().to_pem().unwrap();
        assert!(pem.starts_with("-----BEGIN PRIVATE KEY-----"), "PKCS#8");
        assert_eq!(AuthKey::from_pem(&pem).unwrap().kid(), key().kid());
        let shown = format!("{:?}", key());
        let n = key().jwks()["keys"][0]["n"].as_str().unwrap().to_string();
        assert!(shown.contains(key().kid()), "{shown}");
        assert!(
            !shown.contains("BEGIN") && !shown.contains(&n[..40]),
            "{shown}"
        );
        let e = AuthKey::from_pem("not a key").unwrap_err();
        assert!(e.starts_with("not a PKCS#8 RSA private key"), "{e}");
    }

    #[test]
    fn token_verifies_with_an_independent_verifier() {
        let body = issuer().login("alice", "secret").unwrap();
        let token = body["access_token"].as_str().unwrap();
        let header = jsonwebtoken::decode_header(token).unwrap();
        assert_eq!(header.alg, jsonwebtoken::Algorithm::RS256);
        assert_eq!(header.typ.as_deref(), Some("JWT"));
        assert_eq!(header.kid.as_deref(), Some(key().kid()));
        let set: jsonwebtoken::jwk::JwkSet = serde_json::from_value(key().jwks()).unwrap();
        let dk = jsonwebtoken::DecodingKey::from_jwk(set.find(key().kid()).unwrap()).unwrap();
        let v = verifier("http://localhost:8080");
        let data = jsonwebtoken::decode::<Value>(token, &dk, &v).unwrap();
        assert_eq!(data.claims["sub"], "alice");
        // A changed payload no longer verifies.
        let mut parts: Vec<String> = token.split('.').map(str::to_string).collect();
        parts[1] = URL_SAFE_NO_PAD
            .encode(br#"{"sub":"mallory","iss":"http://localhost:8080","exp":9999999999}"#);
        assert!(jsonwebtoken::decode::<Value>(parts.join("."), &dk, &v).is_err());
    }

    /// B9: the measured size of a login reply is the size of the reply, to the byte, for every
    /// outcome: the oracle is [`respond`]'s own body, serialized as the mock writes it.
    #[test]
    fn a_login_reply_is_sized_exactly_without_signing() {
        let i = issuer();
        // `Credentials` is not `Clone`, so each case is built where it is used
        let case = |n: usize| -> Result<Credentials, Reject> {
            let (u, p) = [
                ("alice", "secret"),
                ("bob", "pw"),
                ("alice", "nope"),
                ("mallory", "x"),
            ][n.min(3)];
            match n {
                4 => Err((415, "unsupported_media_type")),
                5 => Err((400, "invalid_request")),
                _ => Ok(Credentials {
                    username: u.into(),
                    password: p.into(),
                }),
            }
        };
        let names = [
            "scopes and claims",
            "no scopes",
            "wrong password",
            "unknown user",
            "415",
            "400",
        ];
        for (n, what) in names.iter().enumerate() {
            let (_, body) = respond(Responder::Login, Some(&i), Some(case(n)));
            assert_eq!(
                login_reply_len(Some(&i), &case(n)),
                json_len(&body),
                "{what}"
            );
        }
        assert_eq!(
            login_reply_len(None, &case(0)),
            json_len(&respond(Responder::Login, None, Some(case(0))).1),
            "no signing key"
        );
        // alice's token carries more claims and a scope, so her reply is longer than bob's
        assert!(
            i.login_len("alice", "secret") > i.login_len("bob", "pw"),
            "length follows the account"
        );
    }

    #[test]
    fn claims_follow_the_precedence() {
        let i = issuer();
        let c = claims_of(&i.token(&i.users[0], 1_000_000, 0xabc));
        assert_eq!(c["sub"], "alice");
        assert_eq!(c["iss"], "http://localhost:8080");
        assert_eq!(
            (c["iat"].as_u64(), c["exp"].as_u64()),
            (Some(1_000_000), Some(1_007_200))
        );
        assert_eq!(c["jti"], "00000000000000000000000000000abc");
        assert_eq!(c["roles"], json!(["admin"]));
        assert_eq!(c["scope"], "read write");
        assert_eq!(c["aud"], "my-app", "node claim");
        assert_eq!(c["tier"], "pro", "the user's claim wins over the node's");
        assert_eq!(c["email"], "a@x.dev");
        let b = claims_of(&i.token(&i.users[1], 5, 1));
        let names: BTreeSet<&str> = b.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            names,
            BTreeSet::from(["aud", "exp", "iat", "iss", "jti", "roles", "sub", "tier"]),
            "no scope claim without scopes"
        );
        assert_eq!(
            (b["roles"].clone(), b["tier"].clone()),
            (json!([]), json!("free"))
        );
    }

    #[test]
    fn standard_claims_win_even_if_a_config_skipped_the_load_check() {
        // `from_yaml` rejects reserved claim names; a hand-built `Config` does not.
        let mut cfg = Config::from_yaml(CFG).unwrap();
        let NodeConfig::Auth(a) = cfg.nodes.get_mut("login").unwrap() else {
            unreachable!("CFG's node is an auth node")
        };
        a.claims.insert("iss".into(), json!("evil"));
        a.users[0].claims.insert("sub".into(), json!("root"));
        a.users[0].claims.insert("exp".into(), json!(1));
        let i = Issuer::from_config(&cfg, Some(key().clone()), 8080)
            .unwrap()
            .unwrap();
        let c = claims_of(&i.token(&i.users[0], 1_000_000, 1));
        assert_eq!(c["sub"], "alice");
        assert_eq!(c["iss"], "http://localhost:8080");
        assert_eq!(c["exp"], 1_007_200);
    }

    #[test]
    fn login_rejects_unknown_users_and_wrong_passwords() {
        let i = issuer();
        assert!(i.login("alice", "wrong").is_none());
        assert!(i.login("mallory", "secret").is_none());
        assert!(
            i.login("Alice", "secret").is_none(),
            "usernames are case-sensitive"
        );
        assert!(i.login("alice", "").is_none());
        let ok = i.login("alice", "secret").unwrap();
        assert_eq!(ok["token_type"], "Bearer");
        assert_eq!(ok["expires_in"], 7200);
        assert_eq!(ok["scope"], "read write");
        assert_eq!(ok.as_object().unwrap().len(), 4, "{ok}");
        let bob = i.login("bob", "pw").unwrap();
        assert!(bob.get("scope").is_none(), "{bob}");
        // jti is random per token.
        let again = i.login("alice", "secret").unwrap();
        assert_ne!(
            claims_of(ok["access_token"].as_str().unwrap())["jti"],
            claims_of(again["access_token"].as_str().unwrap())["jti"]
        );
    }

    #[test]
    fn from_config_fails_closed_without_a_key() {
        let cfg = Config::from_yaml(CFG).unwrap();
        let e = Issuer::from_config(&cfg, None, 8080).unwrap_err();
        assert_eq!(
            e.msg,
            "auth node 'login' needs a signing key (kaanal up provides one)"
        );
        let none = Config::from_yaml("version: 1\nnodes:\n  app: { type: service }\n").unwrap();
        assert!(Issuer::from_config(&none, None, 8080).unwrap().is_none());
        let bare = Config::from_yaml("version: 1\nnodes:\n  login: { type: auth }\n").unwrap();
        let i = Issuer::from_config(&bare, Some(key().clone()), 1)
            .unwrap()
            .unwrap();
        assert_eq!(
            i.ttl,
            Duration::from_secs(3600),
            "default token_ttl (R1.16)"
        );
        assert_eq!(i.iss, "http://localhost:1", "X18: the bound port");
    }
}
