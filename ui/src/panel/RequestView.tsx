// One request, hop by hop: a waterfall on a shared time axis (spec 6.1). A hop is its wait in line
// (amber) then its work. The finding sentence appears only when the lines carry `wait_ms`; a failed
// request (`wait_ms: null`) lists its hops and the failure, with no wait split. Escape closes.

import { useEffect, useRef, useState } from "react";
import { api } from "../api/client";
import type { LogLine } from "../api/types";
import { eventWords, waterfall } from "./logFormat";

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

  // opened under a long list, it would sit off screen: bring it into view
  const box = useRef<HTMLElement>(null);
  useEffect(() => {
    box.current?.scrollIntoView?.({ block: "nearest" });
  }, [request, stream]);
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

  // Escape closes the open request, wherever focus is inside the tab
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const lines = hops?.key === key ? hops.lines : null;
  const w = lines && lines.length > 0 ? waterfall(lines) : null;

  return (
    <section
      ref={box}
      aria-label={`Request r-${request}`}
      className="mt-3 border-t border-border pt-2"
    >
      <div className="mb-1 flex items-center">
        <h3 className="text-xs font-semibold">
          r-{request} {stream}
        </h3>
        <button type="button" onClick={onClose} className="ml-auto text-xs underline">
          Close
        </button>
      </div>
      {lines === null ? (
        <p className="text-xs text-muted">Loading...</p>
      ) : hops?.problem ? (
        <p role="alert" className="text-xs text-negative">
          {hops.problem}
        </p>
      ) : w === null ? (
        <p className="text-xs text-muted">
          This request is not in the sample of this run (only whole requests that were kept are).
        </p>
      ) : (
        <>
          {w.finding ? <p className="mb-1 text-xs font-medium">{w.finding}</p> : null}
          <ol className="space-y-1 text-xs">
            {w.rows.map(({ line: l, label, left, wait, work }, i) => (
              <li key={i} className="flex items-center gap-2">
                <span className="w-24 truncate font-mono">{label}</span>
                <span className="relative h-3 flex-1 rounded bg-well">
                  {l.wait_ms === null && l.service_ms === null ? (
                    <span
                      className="absolute top-0 h-3 w-1 rounded bg-bad-fill"
                      style={{ left: `${Math.min(left, 99)}%` }}
                    />
                  ) : (
                    <>
                      {wait > 0 ? (
                        <span
                          className="absolute top-0 h-3 rounded-l bg-warn-fill"
                          style={{ left: `${left}%`, width: `${wait}%` }}
                        />
                      ) : null}
                      <span
                        className={`absolute top-0 h-3 rounded-r ${l.level === "error" ? "bg-bad-fill" : "bg-ok-fill"}`}
                        style={{ left: `${left + wait}%`, width: `${Math.max(work, 1)}%` }}
                      />
                    </>
                  )}
                </span>
                <span className="w-36 text-right">{eventWords(l)}</span>
              </li>
            ))}
          </ol>
        </>
      )}
    </section>
  );
}
