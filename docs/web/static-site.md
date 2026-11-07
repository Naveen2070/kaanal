# Serving your frontend: `site:`

Point Kaanal at the folder your frontend builds into, and it serves the page and the mocks from one origin. No CORS setup, no second dev server.

```yaml
version: 1
nodes:
  # ...
site: web # a folder next to kaanal.yaml
```

`kaanal up`, then open `http://127.0.0.1:<port>/`: you get `web/index.html`, and its `fetch("/users")` calls reach the mocks.

## The key

- `site:` is a path **relative to `kaanal.yaml`**, to a folder **inside** the project (not the project folder itself). An absolute path, a path that leaves the project with `..`, or a symlink that points outside it is an error at load time, from `kaanal validate` and `kaanal up` alike.
- Without the key nothing is served from disk.

## What is served

| Request                                | Answer                                                                                       |
| -------------------------------------- | -------------------------------------------------------------------------------------------- |
| `GET /`                                | `index.html`                                                                                 |
| `GET /app.js`, `GET /img/logo.svg`     | That file                                                                                    |
| `GET /shop/` or `GET /shop` (a folder) | `shop/index.html`; a folder without one is not listed                                        |
| `HEAD` of any of the above             | The same headers, no body                                                                    |
| `POST`, `PUT`, ... any path            | Never a file: the mock answers                                                               |
| A path a spec defines (`GET /users`)   | The mock, always. A file with the same path is ignored                                       |
| A path with no file                    | The mock's usual answer: `404 {"error":"no route","path":...}`, logged like any mock request |
| Anything under `/__kaanal/`            | Never a file, in any spelling (encoded, any case), even if the folder has one                |

Every file is sent with `Cache-Control: no-cache` and `X-Content-Type-Options: nosniff`. The content type comes from the extension: `html`, `css`, `js`, `mjs`, `json`, `map`, `txt`, `svg`, `png`, `jpg`, `jpeg`, `gif`, `webp`, `ico`, `woff`, `woff2`; anything else is `application/octet-stream`. Files get no CORS headers: the page is same-origin.

A folder requested without a trailing slash (`/shop`) gets a `308` to `/shop/` (the query string is kept), so the page's relative links resolve inside the folder. The target is always one slash followed by the path's own segments, so `//shop` is `/shop/` and never a link to another host. A folder with no `index.html` is still a 404.

Names that start with a dot (`.env`, `.git/config`) are never served, except under `.well-known`; this is checked on the resolved path too, so a link to one is refused. Paths that try to leave the folder (`..` in any spelling, `\`, a drive letter, a `NUL` byte, a symlink that points outside) are never served; they get the mock's 404.

## Files skip the simulation

Site files do **not** go through the topology: no chaos, latency, queueing or rate limit applies to them, they take no request id, and they do not appear in `kaanal logs`, `kaanal trace` or the event stream. This is on purpose. With the database down, your page and its chaos controls must still load, so you can watch the API calls fail. It is also how real dev servers keep assets apart from the API.
