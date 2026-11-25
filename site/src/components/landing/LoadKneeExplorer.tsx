import { useMemo, useState } from "react";

interface NodeLoad {
  name: string;
  hueVar: string;
  capacityRps: number;
  arrivingRps: number;
  utilization: number;
  meanMs: number;
  note: string;
}

function barFill(util: number): string {
  if (util >= 0.9) return "var(--bad-fill)";
  if (util >= 0.7) return "var(--warn-fill)";
  return "var(--ok-fill)";
}

export function LoadKneeExplorer() {
  const [offeredRps, setOfferedRps] = useState(140);
  const [dbCopies, setDbCopies] = useState(1);
  const [cacheHitPct, setCacheHitPct] = useState(0);

  const metrics = useMemo(() => {
    const cacheBypassFactor = 1 - cacheHitPct / 100;

    // Capacities matching realistic Studio defaults:
    // nginx: 256 slots @ 1ms -> >25k req/s (effectively unbounded here)
    // shop: 8 slots @ 25ms -> 320 req/s
    // cache: 64 slots @ 0.5ms -> 128k req/s
    // db: 4 pool slots per copy @ 32ms read/scan mix -> 125 req/s per copy
    const shopCap = 320;
    const dbCapPerCopy = 125;
    const dbCapTotal = dbCapPerCopy * dbCopies;

    const gwArriving = offeredRps;
    const shopArriving = offeredRps;
    const cacheArriving = cacheHitPct > 0 ? offeredRps : 0;
    const dbArriving = offeredRps * cacheBypassFactor;

    const shopUtil = shopArriving / shopCap;
    const dbUtil = dbArriving / dbCapTotal;
    const gwUtil = gwArriving / 2500;
    const cacheUtil = cacheArriving / 2500;

    // Saturation point in total offered req/s for each node
    const shopKnee = shopCap;
    const dbKnee = cacheBypassFactor > 0 ? dbCapTotal / cacheBypassFactor : Infinity;
    const breaksFirst =
      dbKnee <= shopKnee
        ? { node: "db", at: Math.round(dbKnee) }
        : { node: "shop", at: Math.round(shopKnee) };

    const peakUtil = Math.max(shopUtil, dbUtil);
    const saturated = peakUtil >= 0.95;

    // Queueing estimate (M/M/c-style growth near rho -> 1, capped at queue: 100 + 30s client timeout)
    const rho = Math.min(0.995, peakUtil);
    const baseMs = 38;
    const p99Ms =
      peakUtil >= 1
        ? Math.min(30000, Math.round(1450 + (peakUtil - 1) * 8500))
        : Math.round(baseMs * (1 / Math.pow(1 - rho * 0.9, 1.35)));

    const maxSustainable = Math.min(shopKnee, dbKnee);
    const goodput = Math.round(Math.min(offeredRps, maxSustainable));
    const dropped = Math.max(0, Math.round(offeredRps - maxSustainable));

    const nodes: NodeLoad[] = [
      {
        name: "nginx",
        hueVar: "--cat-gateway",
        capacityRps: 2500,
        arrivingRps: gwArriving,
        utilization: gwUtil,
        meanMs: 1,
        note: "256 slots · 1 ms",
      },
      {
        name: "shop",
        hueVar: "--cat-service",
        capacityRps: shopCap,
        arrivingRps: shopArriving,
        utilization: shopUtil,
        meanMs: 25,
        note: "8 slots · 25 ms",
      },
      {
        name: "cache",
        hueVar: "--cat-cache",
        capacityRps: 2500,
        arrivingRps: cacheArriving,
        utilization: cacheUtil,
        meanMs: 0.5,
        note: cacheHitPct === 0 ? "off (0% hit)" : `${cacheHitPct}% hit · 0.5 ms`,
      },
      {
        name: "db",
        hueVar: "--cat-database",
        capacityRps: dbCapTotal,
        arrivingRps: Math.round(dbArriving),
        utilization: dbUtil,
        meanMs: 32,
        note: `${dbCopies} ${dbCopies === 1 ? "copy" : "copies"} × 4 pool slots · 32 ms`,
      },
    ];

    return {
      breaksFirst,
      saturated,
      p99Ms,
      goodput,
      dropped,
      nodes,
    };
  }, [offeredRps, dbCopies, cacheHitPct]);

  return (
    <div className="sheet sim-panel">
      <div className="sim-top">
        <div className="kpi-strip">
          <div>
            <div className="kpi-label">Breaks first</div>
            <div
              className="kpi-val"
              style={{ color: metrics.saturated ? "var(--negative)" : "var(--text)" }}
            >
              {metrics.breaksFirst.node} at {metrics.breaksFirst.at} req/s
            </div>
          </div>
          <div>
            <div className="kpi-label">Offered load</div>
            <div className="kpi-val">{offeredRps} req/s</div>
          </div>
          <div>
            <div className="kpi-label">System p99</div>
            <div
              className="kpi-val"
              style={{ color: metrics.p99Ms >= 1000 ? "var(--warn)" : "var(--text)" }}
            >
              {metrics.p99Ms >= 1000
                ? `${(metrics.p99Ms / 1000).toFixed(2)} s`
                : `${metrics.p99Ms} ms`}
            </div>
          </div>
          <div>
            <div className="kpi-label">Goodput</div>
            <div className="kpi-val" style={{ color: "var(--positive)" }}>
              {metrics.goodput} /s
            </div>
          </div>
          <div>
            <div className="kpi-label">Turned away (503)</div>
            <div
              className="kpi-val"
              style={{ color: metrics.dropped > 0 ? "var(--negative)" : "var(--muted)" }}
            >
              {metrics.dropped} /s
            </div>
          </div>
        </div>
      </div>

      <div className="sim-controls">
        <div className="ctrl-group">
          <div className="ctrl-head">
            <label htmlFor="sim-rps">Offered load</label>
            <span className="mono">{offeredRps} req/s</span>
          </div>
          <input
            id="sim-rps"
            className="sim-slider"
            type="range"
            min={20}
            max={360}
            step={5}
            value={offeredRps}
            onChange={(e) => setOfferedRps(Number(e.target.value))}
          />
        </div>

        <div className="ctrl-group">
          <div className="ctrl-head">
            <span>Database copies</span>
            <span className="mono">{dbCopies}</span>
          </div>
          <div className="seg-group" role="group" aria-label="Database copies">
            {[1, 2, 4].map((c) => (
              <button
                key={c}
                type="button"
                className="seg-btn"
                aria-pressed={dbCopies === c}
                onClick={() => setDbCopies(c)}
              >
                {c}×
              </button>
            ))}
          </div>
        </div>

        <div className="ctrl-group">
          <div className="ctrl-head">
            <span>Cache hit ratio</span>
            <span className="mono">{cacheHitPct}%</span>
          </div>
          <div className="seg-group" role="group" aria-label="Cache hit ratio">
            {[0, 50, 80].map((pct) => (
              <button
                key={pct}
                type="button"
                className="seg-btn"
                aria-pressed={cacheHitPct === pct}
                onClick={() => setCacheHitPct(pct)}
              >
                {pct}%
              </button>
            ))}
          </div>
        </div>
      </div>

      <div className="meter-grid">
        {metrics.nodes.map((node) => {
          const pct = Math.round(Math.min(100, node.utilization * 100));
          const label =
            node.utilization >= 0.95
              ? "Saturated"
              : `${pct}% busy`;

          return (
            <div key={node.name} className="card meter-card">
              <div className="meter-head">
                <span style={{ display: "inline-flex", alignItems: "center", gap: "0.4rem" }}>
                  <span
                    className="topo-dot"
                    style={{ background: `var(${node.hueVar})` }}
                  />
                  {node.name}
                </span>
                <span
                  className="mono"
                  style={{
                    fontSize: "0.76rem",
                    color: node.utilization >= 0.9 ? "var(--negative)" : "var(--muted)",
                  }}
                >
                  {label}
                </span>
              </div>
              <div className="meter-track">
                <div
                  className="meter-fill"
                  style={{
                    width: `${pct}%`,
                    background: barFill(node.utilization),
                  }}
                />
              </div>
              <div className="meter-sub">
                {node.arrivingRps} arr/s · {node.note}
              </div>
            </div>
          );
        })}
      </div>

      <p style={{ margin: "0.9rem 0 0", fontSize: "0.78rem", color: "var(--faint)" }}>
        Illustrative queueing preview. In Kaanal,{" "}
        <code className="mono">kaanal sim sweep</code> and Studio run the full discrete-event engine over your exact{" "}
        <code className="mono">kaanal.yaml</code>, including queues, timeouts, retries, breakers and cloud cost estimates.
      </p>
    </div>
  );
}
