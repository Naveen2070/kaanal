// One response body the mock would generate for an endpoint: what a route carries, and the size a
// journey's response defaults to. It is the generator's answer for the applied system (the same
// one and the same seeds the mock uses), not a request that went anywhere.

import { useId, useState } from "react";
import { ApiError, api } from "../../data/api/client";
import type { SampleResult } from "../../data/api/types";

/** The part of a body shown; a long one is cut, and the size above it says how big it really is. */
const SHOW_CHARS = 4000;

export function Sample({ endpoints }: { endpoints: string[] }) {
  const id = useId();
  const [endpoint, setEndpoint] = useState("");
  const [seed, setSeed] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<SampleResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const field = "h-7 rounded border border-control bg-sheet px-1 text-text";

  const ask = async () => {
    const n = seed.trim() === "" ? undefined : Number(seed);
    if (n !== undefined && !(Number.isInteger(n) && n >= 0)) {
      setError("The seed is a whole number, 0 or more.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      setResult(await api.sample(endpoint, n));
    } catch (e) {
      setResult(null);
      setError(e instanceof ApiError ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const text = result === null ? "" : JSON.stringify(result.body, null, 2);
  return (
    <div className="flex flex-col gap-1.5 text-xs">
      <p className="text-muted">
        One response the mock would generate for an endpoint, to see what it carries.
      </p>
      <div className="flex flex-wrap items-center gap-1">
        <label htmlFor={`${id}-ep`}>Endpoint</label>
        <select
          name="endpoint"
          id={`${id}-ep`}
          value={endpoint}
          onChange={(e) => setEndpoint(e.currentTarget.value)}
          className={`${field} min-w-0 max-w-full flex-1`}
        >
          <option value="">an endpoint...</option>
          {endpoints.map((k) => (
            <option key={k} value={k}>
              {k}
            </option>
          ))}
        </select>
      </div>
      <div className="flex flex-wrap items-center gap-1">
        <label htmlFor={`${id}-seed`}>Seed</label>
        <input
          name="seed"
          id={`${id}-seed`}
          type="number"
          min={0}
          placeholder="the file's"
          value={seed}
          onChange={(e) => setSeed(e.currentTarget.value)}
          className={`${field} w-24`}
        />
        <button
          type="button"
          disabled={endpoint === "" || busy}
          onClick={() => void ask()}
          className="min-h-7 rounded border border-control bg-sheet px-2 hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-40"
        >
          {busy ? "Asking..." : "Show a sample"}
        </button>
      </div>
      {error ? (
        <p role="alert" className="text-negative">
          {error}
        </p>
      ) : null}
      {result ? (
        <div>
          <p className="text-muted">
            {result.endpoint} answers {result.status}, {result.bytes.toLocaleString("en-US")} bytes.
          </p>
          {result.body === null ? (
            <p className="text-muted">This route has no response schema, so there is no body.</p>
          ) : (
            <pre
              aria-label="Sample body"
              className="m-0 max-h-56 overflow-auto rounded bg-well p-2 font-mono"
            >
              {text.length > SHOW_CHARS ? `${text.slice(0, SHOW_CHARS)}\n... (cut)` : text}
            </pre>
          )}
        </div>
      ) : null}
    </div>
  );
}
