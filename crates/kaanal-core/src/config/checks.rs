//! Cross-field checks that serde cannot place: auth, capacity and locales, reported at their own key.

use super::{Config, IndexMap, Located, MAX_BURST_WINDOW_S, NOT_TEXT, NodeConfig, PathSeg, keys};

/// **Private.** Claim names Kaanal sets in every token (X3).
pub(super) const RESERVED_CLAIMS: [&str; 7] = ["sub", "iss", "exp", "iat", "jti", "roles", "scope"];

/// **Private.** Most claims, roles or scopes in one list (per user; node `claims` too).
pub(super) const MAX_AUTH_ITEMS: usize = 32;

/// **Private.** Longest claim name, string claim value, role, scope, username or password, in bytes.
pub(super) const MAX_AUTH_TEXT: usize = 256;

/// **Private.** Most users on one auth node.
pub(super) const MAX_USERS: usize = 1000;

/// **Private.** RFC 6749 `scope-token`: `1*( %x21 / %x23-5B / %x5D-7E )`.
pub(super) fn is_scope_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| matches!(b, 0x21 | 0x23..=0x5B | 0x5D..=0x7E))
}

/// **Private.** `base` followed by `rest`.
pub(super) fn under(base: &[PathSeg], rest: &[PathSeg]) -> Vec<PathSeg> {
    [base, rest].concat()
}

/// **Private.** Capacity rules serde cannot place: `pool` and `concurrency` together, and a
/// burst window (`burst / rps`) longer than a year, each at its own key or value.
pub(super) fn check_capacity(cfg: &Config) -> Result<(), Located> {
    for (name, node) in &cfg.nodes {
        if node.pool().is_some() && node.concurrency().is_some() {
            return Err(Located::new(
                keys(&["nodes", name.as_str()]),
                format!("node '{name}' sets both pool and concurrency; use one"),
            ));
        }
        if let Some(rl) = node.rate_limit()
            && f64::from(rl.burst.get()) / rl.rps > MAX_BURST_WINDOW_S
        {
            return Err(Located::at_value(
                keys(&["nodes", name.as_str(), "rate_limit"]),
                format!(
                    "node '{name}': rate_limit burst / rps is over 365 days (lower burst or raise rps)"
                ),
            ));
        }
    }
    Ok(())
}

/// **Private.** The auth rules serde would place at the start of `users:` (spec section 3): checked
/// after parsing, each error at its own key or value. Messages never quote a password.
pub(super) fn check_auth(cfg: &Config) -> Result<(), Located> {
    for (name, node) in &cfg.nodes {
        let NodeConfig::Auth(a) = node else { continue };
        let node_path = keys(&["nodes", name.as_str()]);
        let ctx = format!("node '{name}'");
        check_claims(&a.claims, &node_path, &format!("{ctx}: claims"))?;
        if a.users.len() > MAX_USERS {
            return Err(Located::new(
                under(&node_path, &[PathSeg::Key("users".into())]),
                format!("{ctx}: at most {MAX_USERS} users (got {})", a.users.len()),
            ));
        }
        for (i, u) in a.users.iter().enumerate() {
            let user = under(
                &node_path,
                &[PathSeg::Key("users".into()), PathSeg::Index(i)],
            );
            let here = format!("{ctx}: users[{i}]");
            let field = |f: &str| under(&user, &[PathSeg::Key(f.into())]);
            if u.username.is_empty() || u.username.len() > MAX_AUTH_TEXT {
                return Err(Located::at_value(
                    field("username"),
                    format!("{here}.username: must be 1 to {MAX_AUTH_TEXT} bytes"),
                ));
            }
            if let Some(kind) = u.password.strip_prefix(NOT_TEXT) {
                return Err(Located::at_value(
                    field("password"),
                    format!("{here}.password: must be text, not {kind} (put it in quotes)"),
                ));
            }
            if u.password.len() > MAX_AUTH_TEXT {
                return Err(Located::at_value(
                    field("password"),
                    format!("{here}.password: must be at most {MAX_AUTH_TEXT} bytes"),
                ));
            }
            check_list(
                &u.roles,
                &field("roles"),
                &format!("{here}.roles"),
                "must be at most 256 bytes",
                |_| true,
            )?;
            check_list(
                &u.scopes,
                &field("scopes"),
                &format!("{here}.scopes"),
                "must be 1 to 256 printable ASCII characters other than space, '\"' and '\\' (an OAuth scope)",
                is_scope_token,
            )?;
            check_claims(&u.claims, &user, &format!("{here}.claims"))?;
        }
    }
    Ok(())
}

/// **Private.** A `roles` or `scopes` list at `path`: at most 32 entries, each at most 256
/// bytes and accepted by `ok` (`rule` says what that means).
pub(super) fn check_list(
    items: &[String],
    path: &[PathSeg],
    ctx: &str,
    rule: &str,
    ok: fn(&str) -> bool,
) -> Result<(), Located> {
    if items.len() > MAX_AUTH_ITEMS {
        return Err(Located::new(
            path.to_vec(),
            format!(
                "{ctx}: at most {MAX_AUTH_ITEMS} entries (got {})",
                items.len()
            ),
        ));
    }
    for (j, s) in items.iter().enumerate() {
        if s.len() > MAX_AUTH_TEXT || !ok(s) {
            return Err(Located::new(
                under(path, &[PathSeg::Index(j)]),
                format!("{ctx}[{j}]: {rule}"),
            ));
        }
    }
    Ok(())
}

/// **Private.** The `claims` map under `parent`: at most 32 entries; no reserved name; names
/// and string values at most 256 bytes; values a string, a number or a boolean.
pub(super) fn check_claims(
    claims: &IndexMap<String, serde_json::Value>,
    parent: &[PathSeg],
    ctx: &str,
) -> Result<(), Located> {
    let at = under(parent, &[PathSeg::Key("claims".into())]);
    if claims.len() > MAX_AUTH_ITEMS {
        return Err(Located::new(
            at,
            format!(
                "{ctx}: at most {MAX_AUTH_ITEMS} claims (got {})",
                claims.len()
            ),
        ));
    }
    for (k, v) in claims {
        let here = under(&at, &[PathSeg::Key(k.clone())]);
        if RESERVED_CLAIMS.contains(&k.as_str()) {
            return Err(Located::new(
                here,
                format!(
                    "{ctx}: '{k}' is set by Kaanal and cannot be a claim (reserved: {})",
                    RESERVED_CLAIMS.join(", ")
                ),
            ));
        }
        if k.is_empty() || k.len() > MAX_AUTH_TEXT {
            return Err(Located::new(
                here,
                format!("{ctx}: a claim name must be 1 to {MAX_AUTH_TEXT} bytes"),
            ));
        }
        // YAML `.inf`/`.nan` arrive as `null` (serde_json has no non-finite numbers), so they fail here.
        let fits = match v {
            serde_json::Value::String(s) => s.len() <= MAX_AUTH_TEXT,
            serde_json::Value::Number(_) | serde_json::Value::Bool(_) => true,
            _ => false,
        };
        if !fits {
            return Err(Located::at_value(
                here,
                format!(
                    "{ctx}.{k}: must be a string (at most {MAX_AUTH_TEXT} bytes), a number or a boolean"
                ),
            ));
        }
    }
    Ok(())
}

/// **Private.** The shape every `locale:` tag must have (D1.4), for the editor schema.
pub(super) const LOCALE_PATTERN: &str = "^[A-Za-z]{2,3}(-[A-Za-z0-9]{2,8})*$";

/// Whether `s` is a locale tag Kaanal accepts: `^[A-Za-z]{2,3}(-[A-Za-z0-9]{2,8})*$` (`en`,
/// `en-IN`, `zh-Hant-TW`). Checked at load, before a tag can reach a file lookup, so `../x`,
/// `a/b` and `en_IN` never do. `kaanal-data` keeps the same check for pack file names.
pub fn valid_locale_tag(s: &str) -> bool {
    let mut parts = s.split('-');
    let first = parts.next().unwrap_or_default();
    (2..=3).contains(&first.len())
        && first.bytes().all(|b| b.is_ascii_alphabetic())
        && parts.all(|p| (2..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// **Private.** Every `locale:` value has the tag shape, each error at its value. Whether the
/// pack exists is the server's check (it loads `locales/`).
pub(super) fn check_locales(cfg: &Config) -> Result<(), Located> {
    let places = std::iter::once((keys(&["locale"]), String::new(), cfg.locale.as_deref())).chain(
        cfg.nodes.iter().map(|(name, n)| {
            (
                keys(&["nodes", name.as_str(), "locale"]),
                format!("node '{name}': "),
                n.locale(),
            )
        }),
    );
    for (at, ctx, tag) in places {
        if let Some(tag) = tag.filter(|t| !valid_locale_tag(t)) {
            return Err(Located::at_value(
                at,
                format!("{ctx}locale: `{tag}` is not a locale tag (like en-IN or de-DE)"),
            ));
        }
    }
    Ok(())
}
