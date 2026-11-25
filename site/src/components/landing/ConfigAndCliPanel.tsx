import { useState } from "react";

const KAANAL_YAML = `version: 1
seed: 42
port: 8080

nodes:
  nginx:
    type: gateway
    flavor: nginx
    proc: { p50: 1ms, p99: 3ms }
  app:
    type: service
    spec: openapi.yaml
    proc: { p50: 5ms, p99: 20ms }
  db:
    type: database
    flavor: postgres
    ops:
      read:  { p50: 4ms,  p99: 20ms }
      write: { p50: 8ms,  p99: 40ms }
      scan:  { p50: 30ms, p99: 150ms }

edges:
  - { from: nginx, to: app, net: { p50: 1ms, p99: 2ms } }
  - { from: app,   to: db,  net: { p50: 1ms, p99: 2ms } }

endpoints:
  "GET /health": { override: { status: 200, body: { ok: true } } }

traffic:
  steady:
    rps: 80
    journeys:
      - { endpoint: "GET /users/{id}", weight: 1 }`;

interface CliTab {
  id: string;
  label: string;
  caption: string;
  commands: string;
}

const CLI_TABS: CliTab[] = [
  {
    id: "demo",
    label: "Start & trace",
    caption: "Boot a stack from one command and inspect every hop.",
    commands: `# Run the built-in shop with a live chaos bar
kaanal demo

# Or generate kaanal.yaml from your own OpenAPI 3.0 file
kaanal init openapi.yaml
kaanal up -d
curl -i http://localhost:8080/users/42`,
  },
  {
    id: "chaos",
    label: "Chaos & pins",
    caption: "Break a dependency or cycle frontend UI states without restarting.",
    commands: `# Take the database down for 30s, or inject 503s
kaanal node down db --for 30s
kaanal chaos set db --error-rate 100% --error 503

# Cycle an endpoint through loading -> empty -> error -> slow -> ok
kaanal cycle "GET /products"

# Activate a named scenario with a lease
kaanal scenario activate slow-db --ttl 30s`,
  },
  {
    id: "sim",
    label: "Sweep & plan",
    caption: "Find the saturation knee and search for the cheapest holding size.",
    commands: `# Raise load until p99 or errors cross the limit
kaanal sim sweep -p steady --report sweep.md

# Size the topology for 3x load and compare cloud prices
kaanal sim plan -p steady --target 3x --cheapest

# Inspect the last request as a per-hop waterfall
kaanal trace last`,
  },
  {
    id: "ci",
    label: "CI script",
    caption: "Deterministic seed, clean reset between specs, explicit exit codes.",
    commands: `kaanal up -d -c kaanal.yaml
kaanal scenario activate db-timeout

# Deterministic seed: same failure pattern every run
npm test

kaanal reset
kaanal down`,
  },
];

export function ConfigAndCliPanel() {
  const [activeTab, setActiveTab] = useState("demo");
  const [copiedKey, setCopiedKey] = useState<string | null>(null);

  const current = CLI_TABS.find((t) => t.id === activeTab) ?? CLI_TABS[0];

  async function copyText(key: string, text: string) {
    try {
      await navigator.clipboard.writeText(text);
      setCopiedKey(key);
      setTimeout(() => setCopiedKey((prev) => (prev === key ? null : prev)), 1600);
    } catch {
      // Clipboard unavailable in restricted context
    }
  }

  return (
    <div className="split-2">
      <div className="sheet code-panel">
        <div className="code-header">
          <span className="mono">kaanal.yaml</span>
          <button
            type="button"
            className="copy-btn"
            onClick={() => copyText("yaml", KAANAL_YAML)}
          >
            {copiedKey === "yaml" ? "Copied" : "Copy YAML"}
          </button>
        </div>
        <pre className="code-pre">
          <code>{KAANAL_YAML}</code>
        </pre>
      </div>

      <div className="sheet code-panel">
        <div className="cmd-tabs" role="tablist" aria-label="CLI workflow examples">
          {CLI_TABS.map((tab) => (
            <button
              key={tab.id}
              type="button"
              role="tab"
              className="cmd-tab"
              aria-selected={activeTab === tab.id}
              onClick={() => setActiveTab(tab.id)}
            >
              {tab.label}
            </button>
          ))}
        </div>
        <div className="code-header" style={{ background: "var(--sheet)" }}>
          <span style={{ color: "var(--muted)", fontWeight: 500, fontSize: "0.8rem" }}>
            {current.caption}
          </span>
          <button
            type="button"
            className="copy-btn"
            onClick={() => copyText("cli", current.commands)}
          >
            {copiedKey === "cli" ? "Copied" : "Copy"}
          </button>
        </div>
        <pre className="code-pre">
          <code>{current.commands}</code>
        </pre>
      </div>
    </div>
  );
}
