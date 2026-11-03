// The Log tab: a bounded sample of one run, one line per hop (spec 7.4). It follows the newest run,
// pages with "Load more", filters on the server (errors only, instance, endpoint, this component),
// and a click on a line opens that one request as a waterfall. Everything here is virtual time and
// simulated trace events, and the tab says so.

import { useEffect, useState, type ComponentProps, type ReactNode } from "react";
import { ApiError, api } from "../../data/api/client";
import type { LogLine, LogQuery } from "../../data/api/types";
import { useRun } from "../../data/store/run";
import { useSelected } from "../../data/store/view";
import { clock, eventWords, formatLine, hopMs, whoOf } from "./logFormat";
import { RequestView } from "./RequestView";

interface Page {
  /** What this page of lines was fetched for: `runId` and the filters, as text. */
  key: string;
  lines: LogLine[];
  next: number | null;
  total: number;
  note: string;
  /** A message instead of lines: the run is not readable (yet). */
  problem: string | null;
}

const PAGE = 200;

/** A wire as one select value: the two names, which may hold any character but this one. */
const SEP = "\u001f";
const wireKey = (l: { from: string; to: string }) => `${l.from}${SEP}${l.to}`;

export function LogTab() {
  const run = useRun((s) => s.latest);
  const [selected] = useSelected();
  const [errorsOnly, setErrorsOnly] = useState(false);
  const [replica, setReplica] = useState("");
  const [endpoint, setEndpoint] = useState("");
  const [thisNode, setThisNode] = useState(false);
  const [component, setComponent] = useState("");
  const [wire, setWire] = useState("");
  const [page, setPage] = useState<Page | null>(null);
  const [more, setMore] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [open, setOpen] = useState<{ runId: string; stream: string; request: number } | null>(null);
  const dimmed = useRun((s) => s.dimmed);

  const runId = run?.state === "done" ? run.id : null;
  const streams = run?.summary?.streams.map((s) => s.endpoint) ?? [];
  // a filter on something the newest run no longer has is dropped, not left silently on
  const endpointNow = streams.includes(endpoint) ? endpoint : "";
  const replicaNow = /^\d+$/.test(replica) && Number(replica) <= 65535 ? Number(replica) : null;
  // a component or a wire the newest run no longer has is dropped too
  const components = run?.summary?.nodes.map((n) => n.name) ?? [];
  const wires = run?.summary?.links ?? [];
  const componentNow = components.includes(component) ? component : "";
  const wireNow = wires.find((l) => wireKey(l) === wire);
  const query: LogQuery = {
    ...(errorsOnly && { level: "error" as const }),
    ...(replicaNow !== null && { replica: replicaNow }),
    ...(endpointNow !== "" && { endpoint: endpointNow }),
    ...(componentNow !== ""
      ? { node: componentNow }
      : thisNode && selected !== null && { node: selected }),
    ...(wireNow && { from: wireNow.from, to: wireNow.to }),
  };
  const key = `${runId}|${attempt}|${JSON.stringify(query)}`;
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
        setPage({ key, lines: p.lines, next: p.next, total: p.total, note: p.note, problem: null });
      })
      .catch((e: unknown) => {
        if (ctl.signal.aborted) return;
        setPage({ key, lines: [], next: null, total: 0, note: "", problem: problemOf(e) });
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

  const none =
    !errorsOnly &&
    endpointNow === "" &&
    !thisNode &&
    replicaNow === null &&
    componentNow === "" &&
    !wireNow;
  const clearAll = () => {
    setErrorsOnly(false);
    setEndpoint("");
    setThisNode(false);
    setReplica("");
    setComponent("");
    setWire("");
  };
  const count = (n: number) => n.toLocaleString("en-US");

  return (
    <div className="p-3 text-sm">
      {dimmed ? (
        <p className="mb-2 text-xs text-warn">
          Out of date: the newest run could not finish, so this is the last one that did.
        </p>
      ) : null}
      <p className="text-xs font-medium">
        {runId === null ? "No run yet" : `Run ${runId.replace(/^s/, "")}`} · simulated, virtual time
      </p>
      <p className="mb-2 text-xs text-muted">
        {page && !loading && !page.problem
          ? `${count(page.lines.length)} of ${count(page.total)} hops sampled. `
          : ""}
        Simulated trace events, not process output, and only a sample of the run.
      </p>
      <div className="mb-2 flex flex-wrap items-center gap-1.5 text-xs">
        <Chip on={none} onClick={clearAll}>
          All
        </Chip>
        <Chip on={errorsOnly} onClick={() => setErrorsOnly((v) => !v)}>
          Errors only
        </Chip>
        <Chip
          on={thisNode && selected !== null}
          disabled={selected === null}
          title={selected ?? "Select a component first"}
          onClick={() => setThisNode((v) => !v)}
        >
          This component
        </Chip>
        {streams.map((s) => (
          <Chip
            key={s}
            on={endpointNow === s}
            onClick={() => setEndpoint(endpointNow === s ? "" : s)}
          >
            {s}
          </Chip>
        ))}
        <label className="flex items-center gap-1">
          Component
          <select
            value={componentNow}
            onChange={(e) => {
              // a wire is two components: a wire and a component filter do not mix
              setComponent(e.currentTarget.value);
              setWire("");
              setThisNode(false);
            }}
            className="max-w-32 rounded border border-control bg-sheet px-1"
          >
            <option value="">Any</option>
            {components.map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
        </label>
        <label className="flex items-center gap-1">
          Wire
          <select
            value={wireNow ? wire : ""}
            disabled={wires.length === 0}
            title={
              wires.length === 0
                ? "Only a wire with a bandwidth logs transfers, and none has one"
                : undefined
            }
            onChange={(e) => {
              setWire(e.currentTarget.value);
              setComponent("");
              setThisNode(false);
            }}
            className="max-w-40 rounded border border-control bg-sheet px-1 disabled:opacity-50"
          >
            <option value="">Any</option>
            {wires.map((l) => (
              <option key={wireKey(l)} value={wireKey(l)}>
                {l.from} → {l.to}
              </option>
            ))}
          </select>
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
            className="w-14 rounded border border-control bg-sheet px-1"
          />
        </label>
      </div>
      {endpoint !== "" && endpointNow === "" && runId !== null ? (
        <p className="mb-2 text-xs text-warn">
          The newest run has no endpoint “{endpoint}”, so that filter was dropped.
        </p>
      ) : null}

      {/* every state stays inside this frame, so the tab does not jump */}
      <div className="min-h-40 rounded-lg border border-border">
        {runId === null ? (
          <Say>No finished run yet. The log appears after the first run.</Say>
        ) : loading ? (
          <Say>Loading the log...</Say>
        ) : page?.problem ? (
          <div role="alert" className="p-3 text-muted">
            <p>{page.problem}</p>
            <button
              type="button"
              onClick={() => setAttempt((n) => n + 1)}
              className="mt-2 rounded border border-control px-2 py-0.5"
            >
              Retry
            </button>
          </div>
        ) : page && page.lines.length === 0 ? (
          <Say>No lines match these filters.</Say>
        ) : page ? (
          <table className="w-full text-xs">
            <caption className="sr-only">{page.note}</caption>
            <thead className="sticky top-0 bg-sheet text-left text-muted">
              <tr>
                <th scope="col" className="px-2 py-1 font-normal">
                  Time and level
                </th>
                <th scope="col" className="px-1 py-1 font-normal">
                  Component
                </th>
                <th scope="col" className="px-1 py-1 font-normal">
                  Event
                </th>
                <th scope="col" className="px-2 py-1 text-right font-normal">
                  ms
                </th>
              </tr>
            </thead>
            <tbody>
              {page.lines.map((l, i) => {
                const shown =
                  open !== null &&
                  open.runId === runId &&
                  open.stream === l.stream &&
                  open.request === l.request;
                const t = hopMs(l);
                return (
                  <tr key={i} className={`h-11 border-t border-rule ${shown ? "bg-active" : ""}`}>
                    {/* two measures stack in one cell: the time over its level */}
                    <td className="px-2 py-1 align-top">
                      <span className="block whitespace-nowrap tabular-nums">{clock(l.t_ms)}</span>
                      <Level level={l.level} />
                    </td>
                    <td title={whoOf(l)} className="max-w-20 px-1 py-1 align-top font-mono">
                      {/* a wire is two names: stacked, so neither is cut short */}
                      {l.from ? (
                        <>
                          <span className="block truncate">{l.from} →</span>
                          <span className="block truncate">{l.node}</span>
                        </>
                      ) : (
                        <span className="block truncate">{whoOf(l)}</span>
                      )}
                    </td>
                    <td className="px-1 py-1 align-top">
                      <button
                        type="button"
                        aria-label={formatLine(l)}
                        aria-pressed={shown}
                        onClick={() =>
                          setOpen({ runId: runId as string, stream: l.stream, request: l.request })
                        }
                        className="text-left hover:underline focus-visible:outline-2 focus-visible:outline-accent"
                      >
                        r-{l.request} · {eventWords(l)}
                      </button>
                    </td>
                    <td className="px-2 py-1 text-right align-top tabular-nums">
                      {t === null ? "–" : Number(t.toFixed(t < 10 ? 2 : 1)).toLocaleString("en-US")}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        ) : null}
      </div>
      {page && !page.problem && page.next !== null ? (
        <button
          type="button"
          disabled={more}
          onClick={() => void loadMore()}
          className="mt-2 rounded border border-control px-2 py-0.5 text-xs"
        >
          {more
            ? "Loading..."
            : `Load more (${count(page.lines.length)} of ${count(page.total)} shown)`}
        </button>
      ) : null}

      {/* an opened request belongs to the run it was opened in: a newer run closes it */}
      {open && open.runId === runId ? (
        <RequestView {...open} onClose={() => setOpen(null)} />
      ) : null}
    </div>
  );
}

function Chip({
  on,
  children,
  ...rest
}: { on: boolean } & Omit<ComponentProps<"button">, "aria-pressed">) {
  return (
    <button
      type="button"
      aria-pressed={on}
      {...rest}
      className={`rounded-full border px-2 py-0.5 disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-accent ${
        on ? "border-accent bg-active font-medium" : "border-control"
      }`}
    >
      {children}
    </button>
  );
}

const Say = ({ children }: { children: ReactNode }) => <p className="p-3 text-muted">{children}</p>;

const LEVEL: Record<LogLine["level"], string> = {
  info: "border-border text-muted",
  warn: "border-warn text-warn",
  error: "border-negative text-negative",
};

/** The level as a badge; the word is always there, colour only repeats it. */
function Level({ level }: { level: LogLine["level"] }) {
  return <span className={`rounded border px-1 text-[11px] ${LEVEL[level]}`}>{level}</span>;
}

/** A readable message for a log that cannot be shown. A busy run is waiting, not an error. */
function problemOf(e: unknown): string {
  if (e instanceof ApiError) {
    if (e.status === 409) return "Waiting for the run to finish. The log appears when it does.";
    if (e.status === 404) return "That run is no longer kept (only the last eight are).";
    return e.message;
  }
  return String(e);
}
