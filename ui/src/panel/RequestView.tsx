// One request, hop by hop: every log line of that request as a bar on a shared time axis (a
// waterfall). The sample keeps whole requests, so the hops shown are all there were.

import { useEffect, useState } from "react";
import { api } from "../api/client";
import type { LogLine } from "../api/types";

interface Hops {
  key: string;
  lines: LogLine[];
  problem: string | null;
}

export function RequestView({
  runId,
  stream,
  request,
  onClose,
}: {
  runId: string;
  stream: string;
  request: number;
  onClose: () => void;
}) {
  const key = `${runId}|${stream}|${request}`;
  const [hops, setHops] = useState<Hops | null>(null);

  useEffect(() => {
    const ctl = new AbortController();
    api
      .getLog(runId, { endpoint: stream, request, limit: 1000 }, { signal: ctl.signal })
      .then((p) => setHops({ key, lines: p.lines, problem: null }))
      .catch((e: unknown) => {
        if (!ctl.signal.aborted) setHops({ key, lines: [], problem: String(e) });
      });
    return () => ctl.abort();
  }, [runId, stream, request, key]);

  const lines = hops?.key === key ? hops.lines : null;
  const start = lines ? Math.min(...lines.map((l) => l.t_ms)) : 0;
  const end = lines ? Math.max(...lines.map((l) => l.t_ms + (l.service_ms ?? 0))) : 0;
  const span = Math.max(end - start, 0.001);

  return (
    <section aria-label={`Request r-${request}`} className="mt-3 border-t pt-2">
      <div className="mb-1 flex items-center">
        <h3 className="text-xs font-semibold">
          r-{request} {stream}
        </h3>
        <button type="button" onClick={onClose} className="ml-auto text-xs underline">
          Close
        </button>
      </div>
      {lines === null ? (
        <p className="text-xs text-neutral-500">Loading...</p>
      ) : hops?.problem ? (
        <p role="alert" className="text-xs text-red-700">
          {hops.problem}
        </p>
      ) : lines.length === 0 ? (
        <p className="text-xs text-neutral-500">
          This request is not in the sample of this run (only whole requests that were kept are).
        </p>
      ) : lines.length === 0 ? (
        <p className="text-xs text-neutral-500">
          This request is not in the sample of this run (only whole requests that were kept are).
        </p>
      ) : (
        <ol className="space-y-1 text-xs">
          {lines.map((l, i) => (
            <li key={i} className="flex items-center gap-2">
              <span className="w-24 truncate font-mono">
                {l.replica === null ? l.node : `${l.node}#${l.replica}`}
              </span>
              <span className="relative h-3 flex-1 rounded bg-neutral-100">
                <span
                  className={`absolute top-0 h-3 rounded ${
                    l.level === "error"
                      ? "bg-red-500"
                      : l.level === "warn"
                        ? "bg-amber-500"
                        : "bg-emerald-500"
                  }`}
                  style={{
                    left: `${((l.t_ms - start) / span) * 100}%`,
                    width: `${Math.max(((l.service_ms ?? 0) / span) * 100, 1)}%`,
                  }}
                />
              </span>
              <span className="w-28 text-right">
                {l.event}
                {l.service_ms !== null ? ` ${Number(l.service_ms.toFixed(2))}ms` : ""}
              </span>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}
