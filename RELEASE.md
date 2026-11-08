Tag: v0.0.1

# Kaanal v0.0.1 (pre-release)

The first pre-release of Kaanal, a single binary that pretends to be your backend: OpenAPI 3.0 mocks with generated
data, simulated service and database timing, and live chaos.

This is an early build. Expect breaking changes between `0.0.x` releases.

## Live: a continuous simulation in Studio

Studio opens in **Live** (the default; **Batch** keeps the seeded, repeatable run). When the file is valid it runs traffic from its first `traffic:` profile all the time on the engine the mocks run on: sampled request dots move along the wires, a queue piles up at a busy component, drops show a cross, and the strip counts what arrives, what is dropped this second, p50 and p99. One continuous timeline carries a mark for every change you make. A setting you change (cores, slots, copies, bandwidth, rate limits, times, chaos) acts at once on the running system and on any frontend calling the mocks, keeping queues and running work; adding or removing a component or wire waits for Apply and restarts the run. Real speed with Pause (a hidden page pauses after 60 seconds); the timeline keeps 5 minutes, up to 30. The mocks now choose copies by their live queues (`least-conn`, `p2c`) and plan with a clock, so `from_ms` and `detect` apply to real requests. Server-sent events at `/__kaanal/live`.

## CPUs: work slows when more runs than there are cores

Off unless written (`cpu: { cores: N }` on a gateway, service, monolith, database, auth or load balancer; Studio: **CPUs** in a component's Settings). A node's own work (its `proc` and operations) is shared among the requests running it, so with `n` at once each runs at `min(1, N / n)` of its speed: processor sharing, checked against M/G/1 and Erlang C to within about 2%. Slots (`concurrency`) still decide how many are admitted. Waiting on callees, the wire and wait steps are not stretched. The summary gains `cpu_load`, `cpu_capacity_per_s` and `saturates_by`, and Results says **CPU bound**. The live mock shares the cores too: it runs every request through the same engine. Memory, disk and instance presets are planned.

## How failure is costed: retries, abandon, breakers and detect

All off unless written (`retry`, `abandon`, `breaker`, `detect` on an edge). A retry loads the callee again and waits a
jittered, growing backoff; `retry.budget` is gRPC's retry throttling; `abandon: continue` lets a timed-out call's work
run on, which with retries is the metastable retry storm; `breaker` is resilience4j's count-based breaker (a refused
call is a 503 that never reaches the callee, counted as `short_circuited`); `detect` is the delay before a balancer
notices a downed copy (`from_ms` on a scenario entry says when it went down). Circuit breakers, `from_ms` and `detect` act in the
simulator and in the live mock alike: the mock runs every request on the same engine (a refused call is a 503 flagged `CB`) and plans with the time since the system started.

## Studio: easier controls

Settings, Trace, Results and Try live are the four tabs. Controls come in three layers (quick setups, a few basics,
folded groups that show their value when closed, with a jump box), every term has a help button that opens its
glossary entry, Results opens with a plain verdict and shows the retry funnel and breaker timeline, Try live starts
from an Apply button, and a gateway has a **Steps** group that edits each route's chain: calls, cache lookups and
parallel blocks, with a retry of their own on a call. The server's trace marks each retry (`retry=2/3`) and the
draft API has a `swap` operation that exchanges two list items without losing their comments.

## How loss and compression are costed

Both are off unless an edge says `loss` (a probability, 0.01 is 1%) or `compress` (the share of a response kept on the
wire, 0.3 sends 30%). A lossy transfer pays fast retransmits and retransmission timeouts, a long one runs at the TCP
rate of RFC 5348, and a lost SYN waits 1 s; an edge with no `net` pays nothing. Compression only shrinks the bytes on
the wire, so a store's own time is unchanged.

## How a streamed response is costed

A write to a storage node streams its request the same way (the store works while the bytes arrive). A read of a storage node now streams: its bytes cross a wire or card with a `bandwidth` while the store is still
reading, so the call takes the first byte plus the size over the slowest stage, not the stages added. This only moves
figures with a `bandwidth`. `stream: false` on the edge or the `call:` step restores the old sum; `stream: true` turns
it on for both directions of any callee, or `stream: request` / `stream: response` for one.

## How a new connection is costed

An edge with a `net` (its round trip) opens a new connection for 1 call in `keepalive` (default 1000); that call
pays the TCP handshake (1 round trip), TLS if `tls` is set (1 for 1.3, 2 for 1.2), and TCP slow start for a body
that does not fit the first 14,600 B window. Reused connections pay nothing. An edge without `net` pays nothing.
Existing files with a `net` therefore move by about 1 call in 1000; set `keepalive: 1000000` to leave it out.

## How body sizes are costed

Sizes only matter when a topology has a `bandwidth` (on an edge or a node), a `per_kb`, or a storage `throughput`.
Then:

- **Headers count.** Every wire transfer and every HTTP node's `per_kb` carries 600 B on a request and 400 B on a
  response, an estimate of real HTTP headers. Write `header_bytes: 0` to leave them out, or a size, or
  `{ request, response }`, to change them.
- **Direction follows the operation.** A write (`PUT`, an insert) counts the request body, a read (`GET`) and a scan
  count the response body, and a service counts both. This is how HTTP and object stores move data; there is no
  switch for it.
- **A call can carry its own sizes.** `sends:` and `returns:` on an edge or a `call:` step say what that call
  carries (a 5 MB upload that becomes a 200 KB call to storage); without them the incoming size is passed on.
- **Transfers share a pipe fairly** instead of queueing, so a big upload no longer blocks a small call.

A file that has a `bandwidth`, a `per_kb` or a `throughput` therefore reports different times from an earlier
build: the headers add to every transfer, and a write no longer pays for the reply it never sends.
`kaanal validate` also warns when an OpenAPI route takes or returns a file, because the spec does not give its size.

## Install

Download the archive for your system, check it against `SHA256SUMS`, unpack it and run `kaanal --help`.

| System  | Archive                                         |
| ------- | ----------------------------------------------- |
| Linux   | `kaanal-v0.0.1-x86_64-unknown-linux-gnu.tar.gz` |
| macOS   | `kaanal-v0.0.1-aarch64-apple-darwin.tar.gz`     |
| Windows | `kaanal-v0.0.1-x86_64-pc-windows-msvc.zip`      |

Try it with `kaanal demo`.

<!--
Maintainer notes (an HTML comment, so it does not show in the published release):
- The UI build is part of a release: release.yml builds `ui/` and fails before `cargo build` if
  `crates/kaanal-server/ui-dist/{index.html,app.js,app.css}` is missing. Nothing to do by hand.
- [ ] crates.io step, when there is one: `cargo package` drops git-ignored files, so `ui-dist/` must be built
  and included in the package first (Cargo.toml has no `include` list on purpose). Until then, do not publish.
-->
