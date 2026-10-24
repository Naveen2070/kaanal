//! The `site:` key: a folder of static files, validated like every other path in the config.

use std::path::{Path, PathBuf};
use std::time::Duration;

use kaanal_core::Config;
use serde_json::{Value, json};

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("kaanal-site-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A project with one gateway and the given `site:` value (`None`: no key).
fn src(site: Option<&str>) -> String {
    let tail = site.map(|s| format!("site: {s}\n")).unwrap_or_default();
    format!("version: 1\nnodes:\n  gw: {{ type: gateway }}\n{tail}")
}

/// The error as `check` reports it, placed in `kaanal.yaml`.
fn check(dir: &Path, src: &str) -> Result<Vec<String>, String> {
    kaanal_server::check(&Config::from_yaml(src).unwrap(), dir)
        .map_err(|e| format!("kaanal.yaml:{}", e.in_source(src)))
}

#[test]
fn site_must_be_a_relative_folder_inside_the_project() {
    let outer = scratch("inside");
    let dir = outer.join("proj");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(outer.join("elsewhere")).unwrap();
    std::fs::create_dir_all(dir.join("a")).unwrap();
    std::fs::write(dir.join("a-file"), "x").unwrap();
    for bad in [
        "/abs",
        "../x",
        "a/../../x",
        "../elsewhere",
        "a/../../elsewhere",
        ".",
        "\"\"",
        "./",
        "a-file",
        "missing",
    ] {
        let e = check(&dir, &src(Some(bad))).unwrap_err();
        // positioned at the `site:` line (line 4), naming the key
        assert!(e.starts_with("kaanal.yaml:4:"), "{bad}: {e}");
        assert!(e.contains("site"), "{bad}: {e}");
        let typed = bad.trim_matches('"');
        assert!(e.contains(&format!("'{typed}'")), "{bad}: {e}");
    }
}

#[cfg(unix)]
#[test]
fn a_site_symlink_to_outside_is_rejected() {
    let outer = scratch("symlink");
    let dir = outer.join("proj");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(outer.join("elsewhere")).unwrap();
    std::os::unix::fs::symlink(outer.join("elsewhere"), dir.join("web")).unwrap();
    let e = check(&dir, &src(Some("web"))).unwrap_err();
    assert!(e.contains("site") && e.contains("inside"), "{e}");
}

#[test]
fn site_loads_a_folder_and_check_passes() {
    let dir = scratch("ok");
    std::fs::create_dir_all(dir.join("web")).unwrap();
    std::fs::write(dir.join("web").join("index.html"), "<h1>hi</h1>").unwrap();
    check(&dir, &src(Some("web"))).unwrap();
    check(&dir, &src(Some("./web/"))).unwrap();
}

#[test]
fn no_site_key_means_no_site() {
    let dir = scratch("none");
    check(&dir, &src(None)).unwrap();
}

// ---- serving -------------------------------------------------------------------------------

/// Bytes no response may ever contain: the content of the files outside the site folder.
const SECRET: &str = "TOPSECRET";

/// A gateway, a spec service (`GET /users`, `GET /users/{id}`, `POST /orders`) and a database,
/// serving `web/` as the site.
const SERVE: &str = "version: 1
seed: 42
nodes:
  nginx: { type: gateway, proc: 1ms }
  app:   { type: service, spec: openapi.yaml, proc: 1ms }
  db:    { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app, to: db, net: 1ms }
site: web
";

struct Srv {
    /// `127.0.0.1:port`.
    addr: String,
    base: String,
    token: String,
    client: reqwest::Client,
    /// The project folder; `web/` is the site, `secret.txt` sits next to it.
    dir: PathBuf,
}

/// A running server over a fresh project: `web/index.html`, `secret.txt` in the project and one
/// level above it.
async fn start(name: &str) -> Srv {
    let outer = scratch(&format!("serve-{name}"));
    let dir = outer.join("proj");
    let web = dir.join("web");
    std::fs::create_dir_all(&web).unwrap();
    let spec = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/openapi.yaml");
    std::fs::copy(spec, dir.join("openapi.yaml")).unwrap();
    std::fs::write(web.join("index.html"), "<h1>home</h1>").unwrap();
    std::fs::write(dir.join("secret.txt"), SECRET).unwrap();
    std::fs::write(outer.join("secret.txt"), SECRET).unwrap();
    let token = kaanal_server::new_token();
    let opts = kaanal_server::ServeOpts {
        host: "127.0.0.1".parse().unwrap(),
        port: 0,
        token: token.clone(),
        auth_key: None,
    };
    let s = kaanal_server::serve(&Config::from_yaml(SERVE).unwrap(), &dir, opts)
        .await
        .unwrap();
    let addr = format!("127.0.0.1:{}", s.addr.port());
    Srv {
        base: format!("http://{addr}"),
        addr,
        token,
        client: reqwest::Client::new(),
        dir,
    }
}

impl Srv {
    fn web(&self, rel: &str, body: &[u8]) {
        let p = self.dir.join("web").join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    async fn req(&self, m: reqwest::Method, path: &str) -> reqwest::Response {
        self.client
            .request(m, format!("{}{path}", self.base))
            .send()
            .await
            .unwrap()
    }

    async fn get(&self, path: &str) -> reqwest::Response {
        self.req(reqwest::Method::GET, path).await
    }

    async fn ctl(&self, m: reqwest::Method, path: &str, body: Option<Value>) -> reqwest::Response {
        let mut rb = self
            .client
            .request(m, format!("{}{path}", self.base))
            .bearer_auth(&self.token);
        if let Some(b) = body {
            rb = rb.json(&b);
        }
        rb.send().await.unwrap()
    }

    async fn log_len(&self) -> usize {
        let r = self.ctl(reqwest::Method::GET, "/__kaanal/logs", None).await;
        assert_eq!(r.status(), 200);
        r.json::<Value>().await.unwrap()["entries"]
            .as_array()
            .unwrap()
            .len()
    }

    /// One `GET` sent byte for byte: reqwest would resolve `..` and turn `\` into `/` first.
    /// Returns the status and everything after the headers.
    async fn raw(&self, target: &str) -> (u16, String) {
        let (addr, target) = (self.addr.clone(), target.to_string());
        tokio::task::spawn_blocking(move || {
            use std::io::{Read, Write};
            let mut s = std::net::TcpStream::connect(&addr).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let head =
                format!("GET {target} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
            s.write_all(head.as_bytes()).unwrap();
            let mut buf = Vec::new();
            s.read_to_end(&mut buf).unwrap();
            let text = String::from_utf8_lossy(&buf).into_owned();
            let status = text[9..12].parse().unwrap();
            let body = text.split_once("\r\n\r\n").map_or("", |(_, b)| b);
            (status, body.to_string())
        })
        .await
        .unwrap()
    }
}

/// The mock's own 404, exactly as a path with no route gets it.
async fn assert_mock_404(r: reqwest::Response, path: &str) {
    assert_eq!(r.status(), 404, "{path}");
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["error"], "no route", "{path}: {body}");
}

#[tokio::test]
async fn index_html_at_root() {
    let s = start("index").await;
    let r = s.get("/").await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["content-type"], "text/html; charset=utf-8");
    assert_eq!(r.headers()["cache-control"], "no-cache");
    assert_eq!(r.headers()["x-content-type-options"], "nosniff");
    // same origin: no CORS, and not a mock (no request id)
    assert!(r.headers().get("access-control-allow-origin").is_none());
    assert!(r.headers().get("x-request-id").is_none());
    assert_eq!(r.text().await.unwrap(), "<h1>home</h1>");
    // a folder with an index answers with it, with or without the slash
    s.web("shop/index.html", b"shop");
    assert_eq!(s.get("/shop/").await.text().await.unwrap(), "shop");
    assert_eq!(s.get("/shop").await.text().await.unwrap(), "shop");
}

#[tokio::test]
async fn file_by_path_with_content_type() {
    let s = start("types").await;
    for (file, ct) in [
        ("a/page.html", "text/html; charset=utf-8"),
        ("a/style.css", "text/css; charset=utf-8"),
        ("a/app.js", "text/javascript; charset=utf-8"),
        ("a/data.json", "application/json"),
        ("a/logo.svg", "image/svg+xml"),
        ("a/pic.png", "image/png"),
        ("favicon.ico", "image/x-icon"),
        ("a/font.woff2", "font/woff2"),
        ("a/blob.bin", "application/octet-stream"),
        ("a/noext", "application/octet-stream"),
    ] {
        s.web(file, file.as_bytes());
        let r = s.get(&format!("/{file}")).await;
        assert_eq!(r.status(), 200, "{file}");
        assert_eq!(r.headers()["content-type"], ct, "{file}");
        assert_eq!(r.text().await.unwrap(), file);
    }
}

#[tokio::test]
async fn head_has_headers_and_no_body() {
    let s = start("head").await;
    s.web("app.js", b"console.log(1)");
    let r = s.req(reqwest::Method::HEAD, "/app.js").await;
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.headers()["content-type"],
        "text/javascript; charset=utf-8"
    );
    assert_eq!(r.headers()["content-length"], "14");
    assert!(r.bytes().await.unwrap().is_empty());
}

#[tokio::test]
async fn post_is_not_served_by_the_site() {
    let s = start("post").await;
    let r = s.req(reqwest::Method::POST, "/index.html").await;
    assert_mock_404(r, "POST /index.html").await;
}

#[tokio::test]
async fn a_spec_route_beats_a_file_with_the_same_path() {
    let s = start("spec-wins").await;
    s.web("users", b"FILE");
    let r = s.get("/users").await;
    assert_eq!(r.status(), 200);
    assert!(
        r.headers().get("x-request-id").is_some(),
        "the mock answered"
    );
    assert_ne!(r.text().await.unwrap(), "FILE");
    // a method the spec lacks on that path is still the mock's 405, never the file
    s.web("orders", b"FILE");
    let r = s.get("/orders").await;
    assert_eq!(r.status(), 405);
}

#[tokio::test]
async fn unknown_path_gets_the_mock_404() {
    let s = start("unknown").await;
    assert_mock_404(s.get("/nope.js").await, "/nope.js").await;
    assert_mock_404(s.get("/a/b/c").await, "/a/b/c").await;
}

#[tokio::test]
async fn no_directory_listing() {
    let s = start("listing").await;
    s.web("assets/app.css", b"x");
    assert_mock_404(s.get("/assets/").await, "/assets/").await;
    assert_mock_404(s.get("/assets").await, "/assets").await;
}

#[tokio::test]
async fn chaos_on_every_node_does_not_touch_the_site() {
    let s = start("chaos").await;
    for node in ["nginx", "app", "db"] {
        let body = json!({ "error_rate": 1.0, "error_status": 503 });
        let r = s
            .ctl(
                reqwest::Method::PUT,
                &format!("/__kaanal/chaos/{node}"),
                Some(body),
            )
            .await;
        assert_eq!(r.status(), 200, "{node}");
    }
    assert_eq!(s.get("/users").await.status(), 503, "the chaos is live");
    let r = s.get("/").await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.text().await.unwrap(), "<h1>home</h1>");
}

#[tokio::test]
async fn site_requests_are_not_logged() {
    let s = start("nolog").await;
    s.web("app.js", b"x");
    let before = s.log_len().await;
    s.get("/").await;
    s.get("/app.js").await;
    s.req(reqwest::Method::HEAD, "/").await;
    assert_eq!(s.log_len().await, before);
    // a fall-through is the mock's, so it is logged as one
    s.get("/nope").await;
    assert_eq!(s.log_len().await, before + 1);
}

#[tokio::test]
async fn nothing_under_the_control_prefix_is_served() {
    let s = start("control").await;
    s.web("__kaanal/index.html", SECRET.as_bytes());
    s.web("__kaanal/x.txt", SECRET.as_bytes());
    for p in ["/__kaanal/", "/__kaanal/x.txt", "/__kaanal/index.html"] {
        let r = s.get(p).await;
        assert!(r.headers().get("access-control-allow-origin").is_none());
        let status = r.status();
        let body = r.text().await.unwrap();
        assert!(!body.contains(SECRET), "{p}: {body}");
        assert_eq!(status, 404, "{p}: {body}");
    }
    // spellings that dodge the router's exact `/__kaanal/` prefix but decode (or, on a
    // case-insensitive disk, resolve) to the same folder
    for p in [
        "/%5f%5fkaanal/x.txt",
        "/__kaanal%2fx.txt",
        "/__KAANAL/x.txt",
    ] {
        let (status, body) = s.raw(p).await;
        assert!(!body.contains(SECRET), "{p}: {body}");
        assert_eq!(status, 404, "{p}: {body}");
        assert!(body.contains("\"no route\""), "{p}: {body}");
    }
}

#[tokio::test]
async fn names_that_only_resemble_the_control_prefix_are_served() {
    let s = start("resemble").await;
    s.web("__kaanal2.txt", b"one");
    s.web("_kaanal", b"two");
    s.web("a/__kaanal/x.txt", b"three");
    for (p, want) in [
        ("/__kaanal2.txt", "one"),
        ("/_kaanal", "two"),
        ("/a/__kaanal/x.txt", "three"),
    ] {
        assert_eq!(s.raw(p).await, (200, want.into()), "{p}");
    }
}

#[tokio::test]
async fn traversal_never_returns_file_bytes() {
    let s = start("traversal").await;
    s.web("sub/ok.txt", b"ok");
    // bait for a Windows stream name (`file.txt::$DATA` is the file itself there)
    s.web("file.txt", SECRET.as_bytes());
    // the same file by an absolute path, as typed on this OS
    let abs = s
        .dir
        .join("secret.txt")
        .display()
        .to_string()
        .replace('\\', "/");
    let abs = abs.trim_start_matches('/');
    let mut targets: Vec<String> = [
        "/../secret.txt",
        "/../../secret.txt",
        "/sub/../../secret.txt",
        "/%2e%2e/secret.txt",
        "/%2E%2E/%2e%2e/secret.txt",
        "/..%2fsecret.txt",
        "/sub/..%2f..%2fsecret.txt",
        "/%252e%252e/secret.txt",
        "/%252e%252e%252fsecret.txt",
        "/a%5c..%5c..%5csecret.txt",
        "/..%5csecret.txt",
        "/c:/x",
        "/C:%2fsecret.txt",
        "/index.html%00",
        "/x%00",
        "/sub/ok.txt%00.html",
        "/index.html:$DATA",
        "/%2f%2fsecret.txt",
        "/file.txt::$DATA",
        "/file.txt%3a%3a$DATA",
        // Windows device names, with and without an extension
        "/CON",
        "/NUL",
        "/con.txt",
        "/NUL.txt",
        "/sub/NUL",
    ]
    .map(String::from)
    .to_vec();
    targets.push(format!("/{abs}"));
    targets.push(format!("//{abs}"));
    targets.push(format!("/{}", abs.replace('/', "%2f")));
    for t in &targets {
        let (status, body) = s.raw(t).await;
        assert!(!body.contains(SECRET), "{t}: {body}");
        assert_eq!(status, 404, "{t}: {body}");
        assert!(body.contains("\"no route\""), "{t}: {body}");
    }
    // a raw backslash: hyper refuses the request line or the mock answers; never the file
    for t in ["/a\\..\\..\\secret.txt", "/..\\secret.txt"] {
        let (status, body) = s.raw(t).await;
        assert!(!body.contains(SECRET), "{t}: {body}");
        assert!(status == 400 || status == 404, "{t}: {status} {body}");
    }
    // the control: the same helper does read a real file
    assert_eq!(s.raw("/sub/ok.txt").await, (200, "ok".into()));
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlink_out_of_the_site_is_not_followed() {
    let s = start("symlink-out").await;
    let web = s.dir.join("web");
    std::os::unix::fs::symlink(s.dir.join("secret.txt"), web.join("s.txt")).unwrap();
    std::os::unix::fs::symlink(&s.dir, web.join("up")).unwrap();
    for p in ["/s.txt", "/up/secret.txt"] {
        let (status, body) = s.raw(p).await;
        assert!(!body.contains(SECRET), "{p}: {body}");
        assert_eq!(status, 404, "{p}: {body}");
    }
    // a link that stays inside is fine
    std::os::unix::fs::symlink(web.join("index.html"), web.join("home.html")).unwrap();
    assert_eq!(s.get("/home.html").await.status(), 200);
}

#[tokio::test]
async fn dot_names_are_not_served_except_well_known() {
    let s = start("dotnames").await;
    s.web(".env", SECRET.as_bytes());
    s.web(".git/config", SECRET.as_bytes());
    s.web(".well-known/ok.txt", b"ok");
    for p in ["/.env", "/.git/config", "/%2eenv", "/sub/../.env"] {
        let (status, body) = s.raw(p).await;
        assert!(!body.contains(SECRET), "{p} served the file: {body}");
        assert_eq!(status, 404, "{p}");
    }
    assert_eq!(s.raw("/.well-known/ok.txt").await, (200, "ok".to_string()));
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlink_to_a_dot_name_inside_the_site_is_not_served() {
    let s = start("symlink-dot").await;
    s.web(".env", SECRET.as_bytes());
    s.web(".git/config", SECRET.as_bytes());
    let web = s.dir.join("web");
    std::os::unix::fs::symlink(web.join(".env"), web.join("visible.txt")).unwrap();
    std::os::unix::fs::symlink(web.join(".git"), web.join("pub")).unwrap();
    for p in ["/visible.txt", "/pub/config"] {
        let (status, body) = s.raw(p).await;
        assert!(!body.contains(SECRET), "{p}: {body}");
        assert_eq!(status, 404, "{p}");
    }
}
