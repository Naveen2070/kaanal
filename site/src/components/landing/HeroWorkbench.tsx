import { useMemo, useState } from "react";

type NodeId = "nginx" | "shop" | "cache" | "db" | "payments";
type NodeFault = "ok" | "slow" | "down";
type EndpointKey = "GET /users/42" | "GET /products" | "POST /checkout";
type LocaleTag = "en-IN" | "en-US" | "de-DE";
type ScenarioId = "healthy" | "slow-db" | "cache-down" | "db-503" | "payment-timeout";

interface NodeSpec {
  id: NodeId;
  kind: string;
  hueVar: string;
  baseDesc: string;
}

const NODES: NodeSpec[] = [
  { id: "nginx", kind: "gateway", hueVar: "--cat-gateway", baseDesc: "proc ~0.7ms" },
  { id: "shop", kind: "service", hueVar: "--cat-service", baseDesc: "proc ~3.4ms" },
  { id: "cache", kind: "cache", hueVar: "--cat-cache", baseDesc: "redis · 70% hit" },
  { id: "db", kind: "database", hueVar: "--cat-database", baseDesc: "postgres" },
  { id: "payments", kind: "external-api", hueVar: "--cat-external", baseDesc: "timeout 3s" },
];

const SCENARIOS: { id: ScenarioId; label: string; isChaos: boolean }[] = [
  { id: "healthy", label: "Healthy", isChaos: false },
  { id: "slow-db", label: "Slow DB (+450ms)", isChaos: true },
  { id: "cache-down", label: "Cache down (miss → DB)", isChaos: true },
  { id: "db-503", label: "DB 503", isChaos: true },
  { id: "payment-timeout", label: "Payment timeout (504)", isChaos: true },
];

const PAYLOADS: Record<LocaleTag, Record<EndpointKey, unknown>> = {
  "en-IN": {
    "GET /users/42": {
      id: 42,
      firstName: "Siddharth",
      lastName: "Rao",
      email: "siddharth.rao81@example.com",
      phone: "+91 9385785563",
      city: "Kochi",
      state: "Kerala",
      pincode: "682001",
      status: "active",
    },
    "GET /products": [
      { id: 1, name: "Canvas Backpack", kind: "backpack", price: 2499, currency: "INR" },
      { id: 2, name: "Insulated Flask", kind: "bottle", price: 899, currency: "INR" },
    ],
    "POST /checkout": {
      orderId: "ord-04f2-0019",
      status: "paid",
      total: 2499,
      currency: "INR",
      paymentRef: "pay_92k1a8",
    },
  },
  "en-US": {
    "GET /users/42": {
      id: 42,
      firstName: "Maya",
      lastName: "Patel",
      email: "maya.patel24@example.com",
      phone: "+1 512 555 0148",
      city: "Austin",
      state: "TX",
      pincode: "78701",
      status: "active",
    },
    "GET /products": [
      { id: 1, name: "Canvas Backpack", kind: "backpack", price: 48.0, currency: "USD" },
      { id: 2, name: "Insulated Flask", kind: "bottle", price: 22.0, currency: "USD" },
    ],
    "POST /checkout": {
      orderId: "ord-04f2-0019",
      status: "paid",
      total: 48.0,
      currency: "USD",
      paymentRef: "pay_71m4c2",
    },
  },
  "de-DE": {
    "GET /users/42": {
      id: 42,
      firstName: "Lukas",
      lastName: "Weber",
      email: "lukas.weber17@example.com",
      phone: "+49 40 5550192",
      city: "Hamburg",
      state: "HH",
      pincode: "20095",
      status: "active",
    },
    "GET /products": [
      { id: 1, name: "Canvas Backpack", kind: "backpack", price: 44.0, currency: "EUR" },
      { id: 2, name: "Insulated Flask", kind: "bottle", price: 19.5, currency: "EUR" },
    ],
    "POST /checkout": {
      orderId: "ord-04f2-0019",
      status: "paid",
      total: 44.0,
      currency: "EUR",
      paymentRef: "pay_38x9d4",
    },
  },
};

interface Hop {
  label: string;
  traceToken: string;
  ms: number;
  kind: "ok" | "warn" | "bad";
}

function deterministicJitter(seq: number, salt: number, spread: number): number {
  const x = Math.sin(seq * 12.9898 + salt * 78.233) * 43758.5453;
  const frac = x - Math.floor(x);
  return Number(((frac - 0.5) * spread).toFixed(1));
}

function nextFault(current: NodeFault): NodeFault {
  if (current === "ok") return "slow";
  if (current === "slow") return "down";
  return "ok";
}

function faultBadgeText(id: NodeId, fault: NodeFault): string {
  if (fault === "ok") return "healthy";
  if (id === "cache" && fault === "down") return "down → miss";
  if (fault === "slow") return "+450ms";
  return id === "payments" ? "504 timeout" : "503 down";
}

export function HeroWorkbench() {
  const [endpoint, setEndpoint] = useState<EndpointKey>("GET /users/42");
  const [locale, setLocale] = useState<LocaleTag>("en-IN");
  const [seq, setSeq] = useState(1);
  const [cacheBypassed, setCacheBypassed] = useState(true);
  const [faults, setFaults] = useState<Record<NodeId, NodeFault>>({
    nginx: "ok",
    shop: "ok",
    cache: "ok",
    db: "ok",
    payments: "ok",
  });

  const activeScenario: ScenarioId | null = useMemo(() => {
    const { nginx, shop, cache, db, payments } = faults;
    if (nginx === "ok" && shop === "ok" && cache === "ok" && db === "ok" && payments === "ok") {
      return "healthy";
    }
    if (nginx === "ok" && shop === "ok" && cache === "ok" && db === "slow" && payments === "ok") {
      return "slow-db";
    }
    if (nginx === "ok" && shop === "ok" && cache === "down" && db === "ok" && payments === "ok") {
      return "cache-down";
    }
    if (nginx === "ok" && shop === "ok" && cache === "ok" && db === "down" && payments === "ok") {
      return "db-503";
    }
    if (nginx === "ok" && shop === "ok" && cache === "ok" && db === "ok" && payments === "down") {
      return "payment-timeout";
    }
    return null;
  }, [faults]);

  function applyScenario(id: ScenarioId) {
    const base: Record<NodeId, NodeFault> = {
      nginx: "ok",
      shop: "ok",
      cache: "ok",
      db: "ok",
      payments: "ok",
    };
    if (id === "slow-db") {
      base.db = "slow";
      setCacheBypassed(true);
    } else if (id === "cache-down") {
      base.cache = "down";
      setCacheBypassed(true);
    } else if (id === "db-503") {
      base.db = "down";
      setCacheBypassed(true);
    } else if (id === "payment-timeout") {
      base.payments = "down";
      setEndpoint("POST /checkout");
    }
    setFaults(base);
    setSeq((n) => n + 1);
  }

  function toggleNode(id: NodeId) {
    setFaults((prev) => ({ ...prev, [id]: nextFault(prev[id]) }));
    setSeq((n) => n + 1);
  }

  const sim = useMemo(() => {
    const hops: Hop[] = [];
    const usesCache = endpoint === "GET /users/42";
    const usesPayments = endpoint === "POST /checkout";

    const addHop = (label: string, traceToken: string, ms: number, kind: Hop["kind"] = "ok") => {
      hops.push({ label, traceToken, ms: Math.max(0.2, Number(ms.toFixed(1))), kind });
    };

    // 1. Gateway
    const gwMs = 0.7 + deterministicJitter(seq, 1, 0.3) + (faults.nginx === "slow" ? 450 : 0);
    if (faults.nginx === "down") {
      addHop("nginx.error", "nginx.error=503", 0.5, "bad");
      return finish(503, "Service Unavailable", hops, {
        error: "node_down",
        node: "nginx",
        status: 503,
      });
    }
    addHop("nginx.proc", `nginx.proc=${gwMs.toFixed(1)}ms`, gwMs, faults.nginx === "slow" ? "warn" : "ok");

    // 2. Gateway -> Shop net
    addHop("net (gw→shop)", `net=${(0.8 + deterministicJitter(seq, 2, 0.2)).toFixed(1)}ms`, 0.8 + deterministicJitter(seq, 2, 0.2));

    // 3. Shop service
    const shopBase = 3.4 + deterministicJitter(seq, 3, 0.8) + (faults.shop === "slow" ? 450 : 0);
    if (faults.shop === "down") {
      addHop("shop.error", "shop.error=503", 0.4, "bad");
      return finish(503, "Service Unavailable", hops, {
        error: "node_down",
        node: "shop",
        status: 503,
      });
    }
    addHop("shop.proc", `shop.proc=${shopBase.toFixed(1)}ms`, shopBase, faults.shop === "slow" ? "warn" : "ok");

    // 4. Cache (read-by-id only)
    if (usesCache) {
      if (faults.cache === "down") {
        // Kaanal rule: a failing or down cache fails open as a miss and hits DB
        addHop("cache.miss (down)", "cache.miss=0.4ms", 0.4, "warn");
      } else if (!cacheBypassed && seq % 3 !== 1) {
        const hitMs = 0.5 + (faults.cache === "slow" ? 450 : 0);
        addHop("cache.hit", `cache.hit=${hitMs.toFixed(1)}ms`, hitMs, faults.cache === "slow" ? "warn" : "ok");
        return finish(200, "OK", hops, PAYLOADS[locale][endpoint]);
      } else {
        const missMs = 0.4 + (faults.cache === "slow" ? 450 : 0);
        addHop("cache.miss", `cache.miss=${missMs.toFixed(1)}ms`, missMs, faults.cache === "slow" ? "warn" : "ok");
      }
    }

    // 5. Shop -> DB net + DB op
    addHop("net (shop→db)", `net=${(1.1 + deterministicJitter(seq, 4, 0.3)).toFixed(1)}ms`, 1.1 + deterministicJitter(seq, 4, 0.3));
    if (faults.db === "down") {
      addHop("db.error", "db.error=503", 0.6, "bad");
      return finish(503, "Service Unavailable", hops, {
        error: "upstream_failed",
        node: "db",
        status: 503,
      });
    }
    const opName = endpoint === "GET /products" ? "scan" : endpoint === "POST /checkout" ? "write" : "read";
    const baseOp = opName === "scan" ? 31.4 : opName === "write" ? 8.2 : 4.6;
    const dbMs = baseOp + deterministicJitter(seq, 5, 2.2) + (faults.db === "slow" ? 450 : 0);
    addHop(`db.${opName}`, `db.${opName}=${dbMs.toFixed(1)}ms`, dbMs, faults.db === "slow" ? "warn" : "ok");

    // 6. Optional payments external-api on POST /checkout
    if (usesPayments) {
      addHop("net (→pay)", "net=2.4ms", 2.4);
      if (faults.payments === "down") {
        addHop("payments.timeout", "payments.timeout=504", 3000, "bad");
        return finish(504, "Gateway Timeout", hops, {
          error: "deadline_exceeded",
          kind: "timeout",
          node: "payments",
          status: 504,
        });
      }
      const payMs = 42.0 + deterministicJitter(seq, 6, 8) + (faults.payments === "slow" ? 450 : 0);
      addHop("payments.proc", `payments.proc=${payMs.toFixed(1)}ms`, payMs, faults.payments === "slow" ? "warn" : "ok");
    }

    return finish(200, "OK", hops, PAYLOADS[locale][endpoint]);
  }, [endpoint, locale, seq, faults, cacheBypassed]);

  function finish(status: number, reason: string, hops: Hop[], body: unknown) {
    const totalMs = hops.reduce((acc, h) => acc + h.ms, 0);
    const trace = hops.map((h) => h.traceToken).join(",");
    return { status, reason, hops, totalMs, trace, body };
  }

  let runningOffset = 0;

  return (
    <div className="wb-shell" aria-label="Interactive Kaanal request-path workbench">
      <div className="wb-bar">
        <div className="wb-bar-left">
          <label className="wb-label" htmlFor="wb-endpoint">
            Route
          </label>
          <select
            id="wb-endpoint"
            className="wb-select"
            value={endpoint}
            onChange={(e) => {
              setEndpoint(e.target.value as EndpointKey);
              setSeq((n) => n + 1);
            }}
          >
            <option value="GET /users/42">GET /users/42</option>
            <option value="GET /products">GET /products</option>
            <option value="POST /checkout">POST /checkout</option>
          </select>

          <label className="wb-label" htmlFor="wb-locale">
            Locale
          </label>
          <select
            id="wb-locale"
            className="wb-select"
            value={locale}
            onChange={(e) => setLocale(e.target.value as LocaleTag)}
          >
            <option value="en-IN">en-IN</option>
            <option value="en-US">en-US</option>
            <option value="de-DE">de-DE</option>
          </select>
        </div>

        <button
          type="button"
          className="wb-run-btn"
          onClick={() => {
            setCacheBypassed(false);
            setSeq((n) => n + 1);
          }}
        >
          Send request #{seq + 1}
        </button>
      </div>

      <div className="wb-scenarios" role="group" aria-label="Chaos scenarios">
        <span className="wb-label" style={{ marginRight: "0.25rem" }}>
          Scenario:
        </span>
        {SCENARIOS.map((sc) => (
          <button
            key={sc.id}
            type="button"
            className={`pill-btn ${sc.isChaos ? "is-chaos" : ""}`}
            aria-pressed={activeScenario === sc.id}
            onClick={() => applyScenario(sc.id)}
          >
            {sc.label}
          </button>
        ))}
      </div>

      <div className="wb-canvas">
        <div className="wb-canvas-hint">
          <span>Click any node to cycle: healthy → +450ms → down</span>
          <span className="mono">seed: 42 · deterministic</span>
        </div>

        <div className="topo-row">
          {NODES.map((node) => {
            const fault = faults[node.id];
            const activeForRoute =
              (node.id === "cache" && endpoint === "GET /users/42") ||
              (node.id === "payments" && endpoint === "POST /checkout") ||
              node.id === "nginx" ||
              node.id === "shop" ||
              node.id === "db";

            const stateCls =
              !activeForRoute
                ? "state-skip"
                : fault === "ok"
                  ? "state-ok"
                  : fault === "slow"
                    ? "state-warn"
                    : node.id === "cache"
                      ? "state-warn"
                      : "state-bad";

            return (
              <button
                key={node.id}
                type="button"
                className={[
                  "topo-node",
                  !activeForRoute ? "is-inactive" : "",
                  fault === "slow" ? "is-slow" : "",
                  fault === "down" ? "is-down" : "",
                ]
                  .filter(Boolean)
                  .join(" ")}
                onClick={() => toggleNode(node.id)}
                title={`Toggle ${node.id} chaos`}
              >
                <div className="topo-node-head">
                  <span className="topo-badge" style={{ color: `var(${node.hueVar})` }}>
                    <span className="topo-dot" style={{ background: `var(${node.hueVar})` }} />
                    {node.kind}
                  </span>
                </div>
                <div className="topo-name">{node.id}</div>
                <div className="topo-meta">{node.baseDesc}</div>
                <span className={`topo-state ${stateCls}`}>
                  {activeForRoute ? faultBadgeText(node.id, fault) : "not on route"}
                </span>
              </button>
            );
          })}
        </div>
      </div>

      <div className="wb-output">
        <div className="wb-pane">
          <div className="wb-pane-head">
            <span className="wb-label">Per-hop waterfall</span>
            <span
              className={`status-chip ${sim.status >= 500 ? "state-bad" : "state-ok"}`}
            >
              HTTP {sim.status} · {sim.totalMs >= 1000 ? `${(sim.totalMs / 1000).toFixed(2)} s` : `${sim.totalMs.toFixed(1)} ms`}
            </span>
          </div>

          <div className="waterfall">
            {sim.hops.map((hop, idx) => {
              const leftPct = sim.totalMs > 0 ? (runningOffset / sim.totalMs) * 100 : 0;
              const widthPct = sim.totalMs > 0 ? (hop.ms / sim.totalMs) * 100 : 0;
              runningOffset += hop.ms;
              const fill =
                hop.kind === "bad"
                  ? "var(--bad-fill)"
                  : hop.kind === "warn"
                    ? "var(--warn-fill)"
                    : "var(--ok-fill)";

              return (
                <div key={`${hop.label}-${idx}`} className="wf-row">
                  <span className="wf-label" title={hop.label}>
                    {hop.label}
                  </span>
                  <div className="wf-track">
                    <div
                      className="wf-bar"
                      style={{
                        left: `${Math.min(96, leftPct)}%`,
                        width: `${Math.max(2.5, Math.min(100 - leftPct, widthPct))}%`,
                        background: fill,
                      }}
                    />
                  </div>
                  <span className="wf-ms">{hop.ms >= 1000 ? `${(hop.ms / 1000).toFixed(1)}s` : `${hop.ms.toFixed(1)}ms`}</span>
                </div>
              );
            })}
          </div>

          <div className="trace-line">
            <strong>x-kaanal-trace:</strong> {sim.trace}
          </div>
        </div>

        <div className="wb-pane">
          <div className="wb-pane-head">
            <span className="wb-label">Response ({locale})</span>
            <span className="mono" style={{ fontSize: "0.72rem", color: "var(--faint)", whiteSpace: "nowrap" }}>
              r04f2-{String(seq).padStart(4, "0")}
            </span>
          </div>
          <pre className="json-box">{JSON.stringify(sim.body, null, 2)}</pre>
        </div>
      </div>
    </div>
  );
}
