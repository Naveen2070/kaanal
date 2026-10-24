# Start a project: `kaanal init`

`kaanal init` turns an OpenAPI 3.0 file into a project you can start straight away.

```
kaanal init openapi.yaml
kaanal up
```

It writes `kaanal.yaml` in the current folder: a gateway, one service that serves your spec, a database and a cache, with realistic timings for Postgres and Redis. There is no `endpoints:` block: every route in your spec already gets cache-then-database timing. Add per-route behaviour later with `kaanal endpoints --write`.

It also adds `.kaanal/` to `.gitignore` (creating the file if needed, never twice).

## Questions

On a terminal, `init` asks up to three questions, each with its default in brackets. Press Enter to accept:

| Question     | Default                  | Flag        |
| ------------ | ------------------------ | ----------- |
| Service name | `app`                    | `--service` |
| Port         | `8080`                   | `--port`    |
| Locale       | `en-IN` (generated data) | `--locale`  |

A flag answers its question, so it is not asked. With no spec argument on a terminal, `init` also asks for the spec path. The questions go to stderr, so `--json` output stays clean.

A bad typed answer is explained and asked again (three tries, then exit 64). A bad flag value is refused at once with exit 64. At the end of input the default is used.

`--yes` (or any run where stdin is not a terminal, such as CI) takes every default and never waits. With no spec argument and no terminal, `init` stops with exit code 64.

The service name is one lowercase letter, then lowercase letters, digits, `-` or `_`, up to 32 characters, and cannot be `gateway`, `database` or `cache`.

## Safety

- An existing `kaanal.yaml` is never overwritten unless you pass `--force`.
- The file is checked the same way as `kaanal validate` before it appears. If the spec is missing, is not OpenAPI 3.0, or has no paths, `init` exits 1 and leaves your folder unchanged. Errors name `kaanal.yaml`.
- A leftover `kaanal.yaml.tmp` (an interrupted run) is never overwritten: delete it and retry.
- `.gitignore` is only appended to, in its own line ending. If it cannot be read as UTF-8 or cannot be written, `init` leaves it alone, prints a warning on stderr and still succeeds; add `.kaanal/` yourself. Lines such as `.kaanal`, `/.kaanal/` or `.kaanal/*` count as already ignored.

## From a URL

```
kaanal init https://example.com/api/openapi.yaml
```

`init` downloads the spec and saves it next to `kaanal.yaml`, named after the last part of the URL path when that ends in `.yaml`, `.yml` or `.json` (other characters become `_`), otherwise `openapi.yaml`. The config points at that local copy. A name that is a Windows device name (`con`, `nul`, `com1` and so on) or longer than 100 characters also gives `openapi.yaml`. If a file with that name already exists, `init` stops with exit 64 before downloading; `--force` replaces it and the output says so (`Replaced the existing openapi.yaml`; with `--json`, a `replaced` list). While `init` runs, the old file is kept as `<name>.kaanal-old` and deleted at the end; if that backup name is already taken (an interrupted run), `init` stops with exit 64 and leaves both files alone.

The download is deliberately limited:

- One `GET`, no cookies, no credentials. A URL with a user name or password (`https://user:pass@...`) is refused (exit 64).
- `https` for any host. Plain `http` only for `localhost`, `127.0.0.0/8` or `::1`. Other schemes are refused (exit 64).
- At most 3 redirects, each checked by the same rules, and never from `https` down to `http`.
- 10 seconds in total and 10 MB at most.
- The response must be an OpenAPI 3.0 document. An HTML page, other JSON, Swagger 2.0, a non-2xx status, a timeout or an oversized body is exit 1.

Errors name the URL without its query string and never print the response. If anything fails, including the project check after the download, the downloaded file is removed again (a file replaced with `--force` is moved back from its backup), so your folder is as it was. If a cleanup step itself fails, a warning on stderr names the file.

An `https` download honours the usual proxy environment variables (`HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY`). A plain `http` download, which only ever goes to your own machine, never uses a proxy: a proxy would fetch from its own localhost, not yours.

`https` support is the `https` Cargo feature, on by default (rustls with the `ring` crypto provider and the Mozilla root certificates from `webpki-roots`; building it needs a C compiler). A build with `--no-default-features` has no TLS: an `https://` URL is then refused with exit 64; download the file and pass its path instead.

## Output

```
Wrote kaanal.yaml, .gitignore
Next: kaanal up
```

With `--json` the output is `{"written":["kaanal.yaml",".gitignore"]}`. `.gitignore` is listed only when it changed; a spec downloaded from a URL is listed after `kaanal.yaml`, and one that replaced an existing file is also in `"replaced"`.

`init` writes local files, so it refuses to run when `--endpoint` or `KAANAL_ENDPOINT` is set.
