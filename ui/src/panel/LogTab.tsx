// The Log tab: a bounded sample of one run, one line per hop (spec 7.4). It follows the newest run,
// pages with "Load more", filters on the server (errors only, instance, endpoint, this component),
// and a click on a line opens that one request as a waterfall. Everything here is virtual time and
// simulated trace events, and the tab says so.

import { useEffect, useState } from "react";
import { ApiError, api } from "../api/client";
import type { LogLine, LogQuery } from "../api/types";
import { useRun } from "../store/run";
import { useSelected } from "../store/view";
import { formatLine } from "./logFormat";
import { RequestView } from "./RequestView";

interface Page {
  /** What this page of lines was fetched for: `runId` and the filters, as text. */
  key: string;
  lines: LogLine[];
  next: number | null;
  note: string;
  /** A message instead of lines: the run is not readable (yet). */
  problem: string | null;
}

const PAGE = 200;

export function LogTab() {
  const run = useRun((s) => s.latest);
  const [selected] = useSelected();
  const [errorsOnly, setErrorsOnly] = useState(false);
  const [replica, setReplica] = useState("");
  const [endpoint, setEndpoint] = useState("");
  const [thisNode, setThisNode] = useState(false);
  const [page, setPage] = useState<Page | null>(null);
  const [more, setMore] = useState(false);
  const [open, setOpen] = useState<{ runId: string; stream: string; request: number } | null>(null);
  const dimmed = useRun((s) => s.dimmed);

  const runId = run?.state === "done" ? run.id : null;
  const streams = run?.summary?.streams.map((s) => s.endpoint) ?? [];
  // a filter on something the newest run no longer has is dropped, not left silently on
  const endpointNow = streams.includes(endpoint) ? endpoint : "";
  const replicaNow = /^\d+$/.test(replica) && Number(replica) <= 65535 ? Number(replica) : null;
  const query: LogQuery = {
    ...(errorsOnly && { level: "error" as const }),
    ...(replicaNow !== null && { replica: replicaNow }),
    ...(endpointNow !== "" && { endpoint: endpointNow }),
    ...(thisNode && selected !== null && { node: selected }),
  };
  const key = `${runId}|${JSON.stringify(query)}`;
  const loading = runId !== null && page?.key !== key;

  // the first page, again whenever the run or a filter changes; an answer for an older key is dropped
  useEffect(() => {
    if (runId === null) return;
    const ctl = new AbortController();
    api
      .getLog(runId, { ...query, limit: PAGE }, { signal: ctl.signal })
      .then((p) => {
        // an answer that arrives after this effect was replaced belongs to an older key
        if (ctl.signal.aborted) return;
        setPage({ key, lines: p.lines, next: p.next, note: p.note, problem: null });
      })
      .catch((e: unknown) => {
        if (ctl.signal.aborted) return;
        setPage({ key, lines: [], next: null, note: "", problem: problemOf(e) });
      });
    return () => ctl.abort();
    // `query` is rebuilt every render; `key` is what it is made of
    // eslint-disable-next-line react/exhaustive-deps
  }, [key]);

  const loadMore = async () => {
    if (runId === null || page === null || page.next === null || more) return;
    setMore(true);
    try {
      const p = await api.getLog(runId, { ...query, limit: PAGE, after: page.next });
      setPage((cur) =>
        cur && cur.key === key ? { ...cur, lines: [...cur.lines, ...p.lines], next: p.next } : cur,
      );
    } catch (e) {
      setPage((cur) => (cur && cur.key === key ? { ...cur, problem: problemOf(e) } : cur));
    } finally {
      setMore(false);
    }
  };

  return (
    <div className="p-3 text-sm">
      {dimmed ? (
        <p className="mb-2 text-xs text-amber-700">
          Out of date: the newest run could not finish, so this is the last one that did.
        </p>
      ) : null}
      <p className="mb-2 text-xs text-neutral-500">
        Virtual time inside the simulation, not wall-clock time. These are simulated trace events,
        not process output, and only a sample of the run.
      </p>
      <div className="mb-2 flex flex-wrap items-center gap-2 text-xs">
        <label className="flex items-center gap-1">
          <input
            type="checkbox"
            checked={errorsOnly}
            onChange={(e) => setErrorsOnly(e.currentTarget.checked)}
          />
          Errors only
        </label>
        <label className="flex items-center gap-1">
          Instance
          <input
            type="number"
            min={0}
            max={65535}
            value={replica}
            aria-invalid={replica !== "" && replicaNow === null}
            onChange={(e) => setReplica(e.currentTarget.value)}
            className="w-14 rounded border px-1"
          />
        </label>
        <label className="flex items-center gap-1">
          Endpoint
          <select
            value={endpointNow}
            onChange={(e) => setEndpoint(e.currentTarget.value)}
            className="max-w-40 rounded border px-1"
          >
            <option value="">all</option>
            {streams.map((s) => (
              <option key={s} value={s}>
                {s}
              </option>
            ))}
          </select>
        </label>
        <label className="flex items-center gap-1">
          <input
            type="checkbox"
            checked={thisNode && selected !== null}
            disabled={selected === null}
            onChange={(e) => setThisNode(e.currentTarget.checked)}
          />
          Only {selected ?? "the selected component"}
        </label>
      </div>

      {runId === null ? (
        <p className="text-neutral-500">
          No finished run yet. The log appears after the first run.
        </p>
      ) : loading ? (
        <p className="text-neutral-500">Loading the log...</p>
      ) : page?.problem ? (
        <p role="alert" className="text-neutral-600">
          {page.problem}
        </p>
      ) : page && page.lines.length === 0 ? (
        <p className="text-neutral-500">Nothing in the sample matches those filters.</p>
      ) : page ? (
        <>
          <p className="mb-1 text-xs text-neutral-500">{page.note}</p>
          <ul className="font-mono text-xs">
            {page.lines.map((l, i) => (
              <li key={i}>
                <button
                  type="button"
                  onClick={() =>
                    setOpen({ runId: runId as string, stream: l.stream, request: l.request })
                  }
                  title="Show this request hop by hop"
                  className={`w-full text-left hover:bg-neutral-100 ${
                    l.level === "error"
                      ? "text-red-700"
                      : l.level === "warn"
                        ? "text-amber-700"
                        : ""
                  }`}
                >
                  {formatLine(l)}
                </button>
              </li>
            ))}
          </ul>
          {page.next !== null ? (
            <button
              type="button"
              disabled={more}
              onClick={() => void loadMore()}
              className="mt-2 rounded border px-2"
            >
              {more ? "Loading..." : "Load more"}
            </button>
          ) : null}
        </>
      ) : null}

      {/* an opened request belongs to the run it was opened in: a newer run closes it */}
      {open && open.runId === runId ? (
        <RequestView {...open} onClose={() => setOpen(null)} />
      ) : null}
    </div>
  );
}

/** A readable message for a log that cannot be shown. A busy run is waiting, not an error. */
function problemOf(e: unknown): string {
  if (e instanceof ApiError) {
    if (e.status === 409) return "The run is still going. The log appears when it finishes.";
    if (e.status === 404) return "That run is no longer kept (only the last eight are).";
    return e.message;
  }
  return String(e);
}
