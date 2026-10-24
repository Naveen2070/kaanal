//! The `site:` folder: static files served next to the mocks.

use std::path::{Path, PathBuf};

use axum::http::{Method, header};
use axum::response::{IntoResponse, Response};
use percent_encoding::percent_decode_str;

/// A validated static-files folder.
pub struct Site {
    /// The folder's canonical path, inside the project directory.
    root: PathBuf,
}

impl Site {
    /// Opens `rel` (the config's `site:` value) under `base_dir`. It must be a relative path to
    /// a folder that, once symlinks and `..` are resolved, is inside `base_dir`.
    pub fn open(base_dir: &Path, rel: &str) -> Result<Site, String> {
        let p = Path::new(rel);
        if p.is_absolute() || p.has_root() {
            return Err(format!(
                "site: '{rel}' must be a path relative to this file"
            ));
        }
        let joined = base_dir.join(p);
        let root = joined
            .canonicalize()
            .map_err(|e| format!("site: cannot open '{rel}': {e}"))?;
        if !root.is_dir() {
            return Err(format!("site: '{rel}' is not a folder"));
        }
        let base = base_dir
            .canonicalize()
            .map_err(|e| format!("site: {}: {e}", base_dir.display()))?;
        if root == base {
            return Err(format!(
                "site: '{rel}' must be a folder inside the project, not the project itself"
            ));
        }
        if !root.starts_with(&base) {
            return Err(format!("site: '{rel}' must be inside the project folder"));
        }
        Ok(Site { root })
    }

    /// The folder's canonical path.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// The file a URL path names under `root` (canonical), or `None`. The path is percent-decoded
/// once (so `%252e` stays a literal `%2e`); a `\0`, a `\`, a `..` segment or a segment with a `:`
/// (a drive, a Windows stream) refuses it outright. A trailing `/` or a folder means its
/// `index.html`. The result is canonical and checked to be a regular file under `root`, so a
/// symlink that leads out is refused. A folder without `index.html` is `None`: no listing. A
/// first segment of `__kaanal` (any case, after decoding) is the control prefix, never a file.
pub fn safe_join(root: &Path, url_path: &str) -> Option<PathBuf> {
    let decoded = percent_decode_str(url_path).decode_utf8().ok()?;
    if decoded.contains(['\0', '\\']) {
        return None;
    }
    let mut p = root.to_path_buf();
    // Empty and `.` segments are skipped, so `//x` cannot name an absolute path.
    for seg in decoded.split('/').filter(|s| !s.is_empty() && *s != ".") {
        if seg == ".." || seg.contains(':') || hidden(seg) {
            return None;
        }
        p.push(seg);
    }
    if decoded
        .split('/')
        .find(|s| !s.is_empty() && *s != ".")
        .is_some_and(|s| s.eq_ignore_ascii_case("__kaanal"))
    {
        return None;
    }
    // ponytail: a folder without a trailing slash serves its index.html with no redirect, so its
    // relative links resolve against `/`; add a 308 to `<path>/` if a real nested page needs it.
    if decoded.ends_with('/') || p.is_dir() {
        p.push("index.html");
    }
    let real = p.canonicalize().ok()?;
    // The same dot-name rule on the resolved path, so a link or a short name inside the site
    // folder cannot reach `.env` or `.git`.
    let rel = real.strip_prefix(root).ok()?;
    if rel
        .components()
        .any(|c| hidden(&c.as_os_str().to_string_lossy()))
    {
        return None;
    }
    real.is_file().then_some(real)
}

/// **Private.** Dot-names (`.env`, `.git`) are never served; `.well-known` is the one convention
/// a site needs.
fn hidden(seg: &str) -> bool {
    seg.starts_with('.') && seg != ".well-known"
}

/// Serves `url_path` from the site: `GET` and `HEAD` only (hyper drops a `HEAD` body), no CORS
/// (same origin) and no caching beyond `no-cache`. `None` means not a site file: the caller
/// falls through to the mock.
pub async fn serve(site: &Site, method: &Method, url_path: &str) -> Option<Response> {
    if method != Method::GET && method != Method::HEAD {
        return None;
    }
    // Off the async workers, as `tokio::fs` would do (its feature is not enabled here).
    let (root, url_path) = (site.root().to_path_buf(), url_path.to_string());
    let (file, bytes) = tokio::task::spawn_blocking(move || {
        let file = safe_join(&root, &url_path)?;
        // ponytail: GET and HEAD read the whole file into memory; stream the body if large
        // assets appear.
        let bytes = std::fs::read(&file).ok()?;
        Some((file, bytes))
    })
    .await
    .ok()??;
    let headers = [
        (header::CONTENT_TYPE, content_type(&file)),
        (header::CACHE_CONTROL, "no-cache"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
    ];
    Some((headers, bytes).into_response())
}

/// **Private.** The content type by extension; anything else is `application/octet-stream`.
fn content_type(file: &Path) -> &'static str {
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("json" | "map") => "application/json",
        Some("txt") => "text/plain; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}
