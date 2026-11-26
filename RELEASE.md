Tag: v0.1.1

# Kaanal v0.1.1

Kaanal (கானல், "mirage") is a single binary that pretends to be your backend. Give it an OpenAPI 3.0 file and a small YAML topology, and it serves realistic mock data with the timing and failures of the system behind it. Studio, its graphical editor, lets you build that system, break it, run load against it and see where it gives way.

This is the first public release. Versions below 1.0 may change between releases.

## Highlights

- **Mocks that feel real.** Coherent synthetic data in seven built-in locales (or your own pack), deterministic for a given seed, with a per-request latency trace in `X-Kaanal-Trace`.
- **A system, not just endpoints.** Gateways, services, databases, caches, load balancers, stores, auth and external APIs, with slots, queues, rate limits, CPUs, memory, retries, breakers and replicas.
- **Chaos on demand.** Latency, error rates, a node or a link down, from the CLI, the HTTP API or Studio. Named scenarios stack, can be saved from Studio (**Save live**), and reset in one call. Pins force an endpoint to answer as loading, empty, error or slow.
- **Studio.** `kaanal up` prints the link. Live mode runs traffic continuously and shows requests, queues and drops as you change settings; Batch mode gives a seeded, repeatable run; Admin mode puts your real frontend in the numbers and lets you rewrite a route's response. Canvas notes are saved in `kaanal.layout.json`.
- **Find where it breaks.** A load sweep finds the knee and the first node to saturate, and writes a shareable Markdown report with a fingerprint of the design. `kaanal sim plan` sizes the system for a target load, and `--cheapest` searches for the cheapest design that holds it, with a what-if across AWS, GCP and Azure prices (a relative estimate, not a quote).
- **Cost.** Per-node prices from a bundled catalogue of about 3,300 instance types, with managed database, cache and storage classes.
- **Scriptable.** A control API under `/__kaanal/`, `kaanal logs`, `trace`, `status`, `up -d` and `down` for CI, and a JSON output flag on read commands.
- **Local and private.** Binds to 127.0.0.1, token-protected control API, no real data, one instance per directory.

## Install

Download the archive for your system from this release (Linux x86_64, macOS arm64, Windows x86_64) and check it against `SHA256SUMS`. With Rust 1.100 or newer you can also build from source: `cargo install --path crates/kaanal` after `cd ui && pnpm build`.

Then:

```bash
kaanal demo                       # a demo shop with a chaos bar in your browser
kaanal up -c examples/nginx-server/kaanal.yaml
```

The README has a short tour; the full documentation is at <https://kaanal.naveenr.in/docs/>.

## Known limits

- A simulation is a relative what-if on a model, not a prediction of production. Treat price and capacity figures as estimates.
- Studio needs a modern browser; the by-hand accessibility passes (screen reader, narrow window) are still in progress.
- No macOS x86_64 build yet.
