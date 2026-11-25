---
title: Quick start
description: From install to a first broken database in about five minutes.
sidebar:
  order: 2
---

You will start a small shop backend, make a request, break its database on purpose, and put it back. You need Kaanal
[installed](/docs/start/install/) and `curl`.

## 1. Get a project

`kaanal demo --eject` writes the demo shop as ordinary files you can edit:

```bash
kaanal demo --eject shop
cd shop
```

```text
Wrote the demo project (22 files) to .../shop
Next:  cd shop && kaanal up
```

You now have `kaanal.yaml` (the topology: a gateway, a shop service, a database, a cache, an auth node and a payment
service) and `openapi.yaml` (the routes).

## 2. Start it

```bash
kaanal up -d
```

```text
Mocks: http://localhost:8080
UI:    http://localhost:8080/__kaanal/studio?mode=admin#token=<64 hex chars>
Auth:  POST http://localhost:8080/login, JWKS http://localhost:8080/.well-known/jwks.json, 1 user
Log: .../shop/.kaanal/kaanal.log
Stop: kaanal down
```

`-d` runs it in the background. Kaanal uses port 8080, or the next free one up to 8100; use the port it prints.

## 3. Make a request

```bash
curl -i http://localhost:8080/products
```

```http
HTTP/1.1 200 OK
content-type: application/json
x-kaanal-trace: nginx.proc=0.7ms,net=0.5ms,shop.proc=3.5ms,net=1.4ms,db.scan=98.7ms
x-request-id: rbed0-0001

[{"description":"Roomy, rugged and easy to clean.","id":1,"image":"/img/product.svg","kind":"backpack","name":"Canvas Backpack","price":2499}, ...]
```

The data is realistic and the answer took real time. The `x-kaanal-trace` header shows where the time went: the gateway
(`nginx`), the network, the `shop` service and a 98.7 ms database scan. Your client waited for the sum of those.

## 4. Break the database

```bash
kaanal chaos set db --error-rate 100% --error 503
curl -i http://localhost:8080/products
```

```http
HTTP/1.1 503 Service Unavailable
x-kaanal-trace: nginx.proc=0.7ms,net=0.5ms,shop.proc=3.5ms,net=1.4ms,db.error=503
```

Your own numbers will differ a little from the ones above; what matters is the shape of the trace. The request failed at the database hop (`db.error=503`) after spending the time of the hops before it, the way a real
failure does. `kaanal status` shows the node table with the chaos on `db`:

```text
NODE         TYPE         FLAVOR     STATE  CHAOS
nginx        gateway      nginx      up     -
shop         service      -          up     -
auth         auth         -          up     -
db           database     -          up     +0ms, 100% errors (503)
cache        cache        redis      up     -
payments     external-api -          up     -
active scenarios: -
```

## 5. Put it back

```bash
kaanal chaos clear
kaanal down
```

## Next steps

- **Frontend or mobile developer:** [pin a loading, empty or error state](/docs/guides/break/pins/) and run
  [scenarios](/docs/guides/break/scenarios/) such as `slow-db` (the demo ships fifteen; `kaanal scenario list` shows
  them).
- **CI or test author:** [run it in the background and script it](/docs/guides/observe/lifecycle/).
- **Learning system design:** read [how Kaanal works](/docs/how-it-works/), then draw a small system in Studio and change
  one thing at a time (a cache, a replica, a retry) to see what it does to the trace.
- **Sizing a backend:** open Studio, draw your system and [find where it breaks](/docs/guides/simulate/simulation/).
- **See it first:** [the demo shop](/docs/start/tour/) in your browser, with a chaos bar.
