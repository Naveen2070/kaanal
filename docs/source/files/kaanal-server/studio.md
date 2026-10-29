# `crates/kaanal-server/src/control/studio.rs` and `crates/kaanal-server/build.rs`

Serves Studio's built bundle, or the status page when there is none. User page: [../../../web/studio.md](../../../web/studio.md).

## What it does

`build.rs` checks `ui-dist/index.html`, `app.js` and `app.css`. If all three exist it sets `cfg(kaanal_ui)` (declared with `cargo::rustc-check-cfg`); otherwise it prints a `cargo:warning`. It re-runs when `ui-dist` changes and never runs Node, npm or `vp`.

## Items

| Item        | Kind     | What it does                                                                                                                                                                                                                                                                           |
| ----------- | -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `studio()`  | async fn | `GET /__kaanal/studio`. With `kaanal_ui`: the embedded `index.html` (`text/html`) with a page CSP (`script-src 'self'`, `style-src 'self' 'unsafe-inline'`, `connect-src 'self'`, `frame-ancestors 'none'`) and `Referrer-Policy: no-referrer`. Without: `admin::ui`, the status page. |
| `app_js()`  | async fn | `GET /__kaanal/studio/assets/app.js` (`text/javascript`). Without `kaanal_ui`: 404.                                                                                                                                                                                                    |
| `app_css()` | async fn | `GET /__kaanal/studio/assets/app.css` (`text/css`). Without `kaanal_ui`: 404.                                                                                                                                                                                                          |
| `asset(..)` | fn       | **Private**, `kaanal_ui` only: one embedded file with its content type, plus the page headers for `index.html`.                                                                                                                                                                        |

The three routes are exempt from the token in [guard.rs](control.md) (exact paths, R5.2); the guard's other headers (`no-store`, `nosniff`) still apply. Tests: `tests/site_ui.rs`.
