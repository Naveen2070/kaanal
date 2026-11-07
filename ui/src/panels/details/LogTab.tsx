// The Log tab: a bounded sample of one run, grouped per request (spec 7.4). Each request is one block
// whose header opens and closes it; its hops sit beneath, indented by call depth, each with a type
// badge and a wait-versus-work bar on the request's own axis. It follows the newest run, pages with
// "Load more", and filters on the server (errors only, instance, endpoint, component, wire). Every
// time here is virtual and every line a simulated trace event, and the tab says so.

import { ColumnHeads, Legend } from "./log/Legend";
import { PanelBottomOpen } from "lucide-react";
import { useEffect, useMemo, useState, useRef, type ReactNode } from "react";
import { ApiError, api } from "../../data/api/client";
import type { LogLine, LogQuery } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { useRun } from "../../data/store/run";
import { useSelected } from "../../data/store/view";
import { useLogDrawer } from "./logDrawer";
import { callDepths } from "./logFormat";
import { groupRequests, sampleFinding } from "./log/groups";
import { RequestBlock } from "./log/RequestBlock";
import { Toolbar, type FilterState } from "./log/Toolbar";

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
/** How many requests start open: enough to show the shape, few enough to leave the list scannable. */
const OPEN_FIRST = 5;

/** A wire as one select value: the two names, which may hold any character but this one. */
const SEP = "\u001f";
const wireKey = (l: { from: string; to: string }) => `${l.from}${SEP}${l.to}`;

const BTN =
  "rounded-md border border-control bg-sheet px-2 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent";

export function LogTab({ drawer = false }: { drawer?: boolean } = {}) {
  const out = useLogDrawer((s) => s.out);
  const setOut = useLogDrawer((s) => s.set);
  const run = useRun((s) => s.latest);
  const [selected] = useSelected();
  const edges = useDraft((s) => s.view?.doc?.edges);
  const [errorsOnly, setErrorsOnly] = useState(false);
  const [replica, setReplica] = useState("");
  const [endpoint, setEndpoint] = useState("");
  const [thisNode, setThisNode] = useState(false);
  const [component, setComponent] = useState("");
  const [wire, setWire] = useState("");
  // a request number, typed as 12 or r-12: keeps the lines of requests whose number contains it
  const [reqText, setReqText] = useState("");
  const [filtersOpen, setFiltersOpen] = useState(false);
  const [page, setPage] = useState<Page | null>(null);
  const [more, setMore] = useState(false);
  const [attempt, setAttempt] = useState(0);
  // a request the person opened or closed by hand; the rest follow the default (the first few open)
  const [toggled, setToggled] = useState<Record<string, boolean>>({});
  const dimmed = useRun((s) => s.dimmed);
  // how deep each component sits in the call graph; no readable graph means every hop at depth 0
  const depths = useMemo(() => callDepths(edges), [edges]);

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
    setToggled({});
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

  const digits = reqText.replace(/\D/g, "");
  // the groups are made from every loaded line, so a request cut by a page boundary joins up
  const groups = useMemo(
    () =>
      groupRequests(
        runId ?? "",
        (page?.lines ?? []).filter((l) => digits === "" || String(l.request).includes(digits)),
      ),
    [page?.lines, runId, digits],
  );
  const finding = page && !loading && !page.problem ? sampleFinding(groups) : null;

  const on = [
    errorsOnly && "Errors only",
    digits !== "" && `Request ${digits}`,
    thisNode && selected !== null && "This component",
    endpointNow !== "" && endpointNow,
    componentNow !== "" && `Component ${componentNow}`,
    wireNow && "Wire",
    replicaNow !== null && `Instance ${replicaNow}`,
  ].filter((x): x is string => typeof x === "string");
  const clearAll = () => {
    setErrorsOnly(false);
    setEndpoint("");
    setThisNode(false);
    setReplica("");
    setComponent("");
    setWire("");
    setReqText("");
  };
  const count = (n: number) => n.toLocaleString("en-US");

  const filters: FilterState = {
    errorsOnly,
    setErrorsOnly,
    reqText,
    setReqText,
    thisNode,
    setThisNode,
    selected,
    endpoint: endpointNow,
    setEndpoint,
    streams,
    component: componentNow,
    components,
    // a wire is two components: a wire and a component filter do not mix
    setComponent: (v) => {
      setComponent(v);
      setWire("");
      setThisNode(false);
    },
    wire: wireNow ? wire : "",
    wires: wires.map((l) => ({ key: wireKey(l), label: `${l.from} → ${l.to}` })),
    setWire: (v) => {
      setWire(v);
      setComponent("");
      setThisNode(false);
    },
    replica,
    replicaBad: replica !== "" && replicaNow === null,
    setReplica,
  };

  // Escape closes the Filters panel; otherwise it collapses the request the focus is in
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || !root.current?.contains(e.target as Node)) return;
      if (filtersOpen) {
        setFiltersOpen(false);
        return;
      }
      const k = (e.target as HTMLElement).closest<HTMLElement>("[data-group]")?.dataset.group;
      if (k) setToggled((t) => ({ ...t, [k]: false }));
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [filtersOpen]);

  // popped out: the list is in the drawer, so the rail says where it went
  if (out && !drawer) {
    return (
      <div className="p-3 text-sm">
        <p className="mb-2 text-muted">The log is open along the bottom of the window.</p>
        <button type="button" onClick={() => setOut(false)} className={`${BTN} min-h-8`}>
          Bring it back here
        </button>
      </div>
    );
  }

  return (
    <div ref={root} className="p-3 text-sm">
      {dimmed ? (
        <p className="mb-2 text-xs text-warn">
          Out of date: the newest run could not finish, so this is the last one that did.
        </p>
      ) : null}
      <div className="flex items-start justify-between gap-2">
        <p className="text-xs font-medium">
          {runId === null ? "No run yet" : `Run ${runId.replace(/^s/, "")}`} · simulated, virtual
          time
        </p>
        {drawer ? null : (
          <button
            type="button"
            title="Open the log along the bottom of the window, where it has room"
            onClick={() => setOut(true)}
            className={`${BTN} flex shrink-0 items-center gap-1 py-0.5`}
          >
            <PanelBottomOpen size={12} aria-hidden />
            Pop out
          </button>
        )}
      </div>
      {finding ? <p className="mt-1 text-sm font-medium">{finding}</p> : null}
      <p className="mt-1 mb-2 text-xs text-muted">
        {page && !loading && !page.problem
          ? `${count(page.lines.length)} of ${count(page.total)} hops sampled. `
          : ""}
        Simulated trace events, not process output, and only a sample of the run.
      </p>
      <Toolbar
        f={filters}
        open={filtersOpen}
        setOpen={setFiltersOpen}
        onClear={clearAll}
        anyOn={on.length > 0}
      />
      {endpoint !== "" && endpointNow === "" && runId !== null ? (
        <p className="mb-2 text-xs text-warn">
          The newest run has no endpoint “{endpoint}”, so that filter was dropped.
        </p>
      ) : null}

      {page && !page.problem && groups.length > 0 ? <Legend /> : null}
      {/* every state stays inside this frame, so the tab does not jump */}
      <div className="@container min-h-40 overflow-hidden rounded-lg border border-border">
        {runId === null ? (
          <Say>No finished run yet. The log appears after the first run.</Say>
        ) : loading ? (
          <output aria-label="Loading the log" className="block">
            {[0, 1, 2, 3, 4].map((i) => (
              <div key={i} aria-hidden className="m-2 h-6 rounded bg-well" />
            ))}
          </output>
        ) : page?.problem ? (
          <div role="alert" className="p-3 text-muted">
            <p>{page.problem}</p>
            <button
              type="button"
              onClick={() => setAttempt((n) => n + 1)}
              className={`${BTN} mt-2 min-h-8`}
            >
              Retry
            </button>
          </div>
        ) : page && groups.length === 0 ? (
          on.length > 0 ? (
            <Say>
              No hops match. Clear {on.join(", ")} to see more.
              <button type="button" onClick={clearAll} className="ml-2 underline">
                Clear filters
              </button>
            </Say>
          ) : (
            <Say>This run kept no requests in its sample.</Say>
          )
        ) : page ? (
          <>
            <p className="sr-only">{page.note}</p>
            <ColumnHeads />
            <ol className="m-0 list-none p-0">
              {groups.map((g, i) => (
                <RequestBlock
                  key={g.key}
                  group={g}
                  depths={depths}
                  open={toggled[g.key] ?? i < OPEN_FIRST}
                  onToggle={() =>
                    setToggled((t) => ({ ...t, [g.key]: !(t[g.key] ?? i < OPEN_FIRST) }))
                  }
                />
              ))}
            </ol>
          </>
        ) : null}
      </div>
      {page && !page.problem && page.next !== null ? (
        <button
          type="button"
          disabled={more}
          onClick={() => void loadMore()}
          className={`${BTN} mt-2 min-h-8`}
        >
          {more
            ? "Loading..."
            : `Load more (${count(page.lines.length)} of ${count(page.total)} shown)`}
        </button>
      ) : null}
    </div>
  );
}

const Say = ({ children }: { children: ReactNode }) => <p className="p-3 text-muted">{children}</p>;

/** A readable message for a log that cannot be shown. A busy run is waiting, not an error. */
function problemOf(e: unknown): string {
  if (e instanceof ApiError) {
    if (e.status === 409) return "Waiting for the run to finish. The log appears when it does.";
    if (e.status === 404) return "That run is no longer kept (only the last eight are).";
    return e.message;
  }
  return String(e);
}
