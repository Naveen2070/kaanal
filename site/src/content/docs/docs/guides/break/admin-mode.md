---
title: "Admin mode: your frontend in the live numbers, and rewriting a response"
sidebar:
  order: 4
---

Studio has three modes beside the Apply button: **Live**, **Batch**, and **Admin**. Live runs generated traffic; Batch runs once, seeded and repeatable.

**Admin** is the live view with your own frontend (or `curl`/tests) included in the numbers alongside generated traffic, plus live response rewriting. Nothing needs configuring — keep using the mock's port.

## What you see

- **The strip:** shows live totals plus a split (`Real 12 / s`, `Generated 80 / s`). Toggle **Real only** to filter strip and card numbers to real requests alone (shared queues/slots stay system-wide).
- **The canvas** draws a real request as a **ring** and a generated one as a dot, so the two differ by shape, never only by colour.
- **The Log tab** becomes the **request feed**: what the mock answered, newest first, with the status each request got. It is polled once a second while the tab is open and the page is visible. A request an edit answered says **edited** in words.

A real request goes through the same simulated system as a generated one. It waits in the same queues, takes the same time at each component and meets the same chaos, so a frontend sees what a user would see when the system is loaded.

## Rewriting what a route answers

Press **Edit response** on any feed row to set **status**, **headers**, and **body** (JSON, plain text/HTML/XML/CSV, or base64 **binary** with custom `Content-Type`):

| Button                | What it does                                                                                                                                       |
| --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Apply now**         | Answers from the next request on (in memory; marked _Live only, not in the file_).                                                                 |
| **Back to normal**    | Removes the edit.                                                                                                                                  |
| **Stage in the file** | Writes `override:` into the draft (then **Save** writes `kaanal.yaml`). If `endpoints:` points to another file, writes that file directly instead. |

Validation flags invalid status (`200`–`599`), invalid JSON, bodies on `204`/`205`/`304`, reserved Kaanal/connection headers, header values `> 4096 B`, bodies `> 1 MiB`, or `> 32` headers.

Tick **Answer one request only** to make the edit a one-shot: the next request gets it, then the route is back to normal. A one-shot is live only (it is not staged into the file).

### Who wins

A route answers, from the top:

1. **Chaos** (a node that is down, an error rate, a broken connection). A failing route keeps failing under your edit, so you can test failures with it in place.
2. **A pin** that forces `error` or `empty`. A pin of `loading`, `slow` or `success` lets the edit through.
3. **Your edit.**
4. The route's `override:` in the file.
5. The generated body.

Three things never take an edit: a route that reads a [collection](/docs/guides/mock/collections/) (an edit would freeze live data), the built-in login and key-set routes of an [auth](/docs/guides/mock/auth/) component, and a `HEAD` made from its `GET` (edit the `GET`; the `HEAD` follows).

A request that names a [scenario scope](/docs/guides/break/scenarios/) (`X-Kaanal-Scenario`) never sees an edit, the same as it never sees live pins, so parallel test workers stay apart.

### How long an edit lives

- **Parameter edits** in Studio keep your edits. An **Apply** keeps the edits whose route still takes one and drops the rest, and says which in the reply (`payloads_dropped`).
- **`POST /__kaanal/reset`** clears every edit.
- A restart of the server loses edits that were not staged and saved.
- If `endpoints:` in your `kaanal.yaml` names another file, **Stage in the file** writes that file itself (the draft holds only `kaanal.yaml`), refusing if the file changed while it was written; a one-shot edit cannot be staged.

A header an edit (or an `override:`) sets is listed in `Access-Control-Expose-Headers`, so a browser frontend on another origin can read it with `fetch`.

## From a script

The editor uses the control API ([Control API](/docs/reference/control-api/payload-edits/)): `PUT /__kaanal/payload` with `{ "route", "status", "body", "headers" }`, `DELETE /__kaanal/payload?route=...`, `GET /__kaanal/payload`.
