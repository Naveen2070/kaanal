# The status page (`/__kaanal/ui`)

A single self-contained HTML page served by the binary: a health dashboard for the running instance. It needs **no token** and makes **one call**, `GET /__kaanal/health`, every two seconds from your browser (every ten while the tab is hidden). It follows your system's light or dark theme and uses the same palette as Studio.

`kaanal up` no longer prints this address. It prints the Studio link; the status page stays served for a quick look at whether the instance is up, and for anything that wants a page to point at without a token.

## What it shows

| Part                                 | Meaning                                                                                                                                                                                                                        |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Healthy / Checking / Unreachable** | The state, in words and in a shape (a pulsing dot, a plain dot, a square), announced to a screen reader only when it changes. Two missed checks in a row make it Unreachable; it says how long it has been silent.             |
| **Uptime**                           | How long the process has run, from the last answer and counted on locally between answers; shown as `-` while unreachable.                                                                                                     |
| **Version**                          | The `kaanal` version.                                                                                                                                                                                                          |
| **System**                           | "Project loaded", or "Blank: none yet" for `up --blank` / `kaanal studio` before the first Apply.                                                                                                                              |
| **Answer time**                      | The last round trip, and a small chart of the last minute: a line for answers, red ticks for misses, with the fastest and slowest.                                                                                             |
| **Open Studio**                      | A link to `/__kaanal/studio`, on the Admin tab when a project is loaded. It carries the token from this page's address (`#token=…`) when it has one. Without it the page says Studio needs the token that `kaanal up` printed. |

The page shows nothing a token guards: no node names, endpoints, requests or figures. What the health route says without a token is only `ok`, `version`, `uptime_s` and `system` (see [control-api.md](control-api.md)).

## When Studio is not built in

A binary compiled without the Studio bundle (no `ui/` build before `cargo build`) answers `/__kaanal/studio` with this same page and a notice, "This build has no Studio", saying how to build it. The Open Studio button is then off.

## Security rules the page follows

These are enforced by static tests in `crates/kaanal-server/tests/http/events_ui.rs`:

- The page asks only `/__kaanal/health`: no event stream, no bearer header, no control route, no storage of any kind.
- Every value from the server goes through `textContent`; no `innerHTML` or friends, no `eval`, no inline event handlers, no external URLs.
- A token in the address fragment (never sent to a server) is only copied onto the Studio link and is never stored or logged.
- Served with a strict CSP (inline script and style only, `connect-src 'self'`, `frame-ancestors 'none'`) and `Referrer-Policy: no-referrer`.
- It works without colour (words and shapes), respects `prefers-reduced-motion` and forced colours, and fits a phone.

> Contributor pointer: the page source is `crates/kaanal-server/src/ui.html`, served by `control::ui` with its CSP headers; the health route is `control::health`. See [../source/files/kaanal-server/control.md](../source/files/kaanal-server/control.md).
