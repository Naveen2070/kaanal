// The "Find where it breaks" group of Results (step 9b): the two limits, a Start button, the points
// arriving on the chart while the sweep runs, then the finding and the report the store kept. The
// sweep shares the simulator's one slot with the auto-run loop: a busy answer is a state, never an
// error. The server sweeps the applied design (not the draft) with its first profile, so the body
// names none. A finished sweep from another generation or server process than the one in force says so.

import { useId, useState } from "react";
import { useDraft } from "../../data/store/draft";
import { draftBody } from "../../data/store/loop";
import { isLive, useMode } from "../../data/store/mode";
import { useRun } from "../../data/store/run";
import { useSweep } from "../../data/store/sweep";
import { twoSignificant } from "../../canvas/metrics";
import { LazyBoundary } from "../../kit/LazyBoundary";
import { Group } from "../details/groups/Group";
import { RESULTS_GROUP } from "../details/groups/ids";
import { CopySaveButtons } from "../details/ResultsExtras";
import { SweepChart } from "./SweepChart.lazy";
import { describeSweep } from "./sweepMath";

const input = "h-7 w-24 rounded border border-control bg-sheet text-text px-1";

/** Highest p99 limit taken, ms (about 11.6 days): past it the number is a typo, not a limit. */
const MAX_P99_MS = 1e9;

/** Why the p99 field cannot be sent, or null when it can. */
function p99Problem(text: string): string | null {
  if (text.trim() === "") return "Type a p99 limit in ms.";
  const n = Number(text);
  return Number.isFinite(n) && n > 0 && n <= MAX_P99_MS
    ? null
    : "The p99 limit must be above 0 ms and at most 1,000,000,000 ms.";
}

/** Why the error field cannot be sent, or null when it can. */
function errProblem(text: string): string | null {
  if (text.trim() === "") return "Type an error limit in %.";
  const n = Number(text);
  return Number.isFinite(n) && n >= 0 && n <= 100 ? null : "The error limit must be 0 to 100 %.";
}

export function SweepGroup() {
  const ids = { p99: useId(), err: useId(), p99Bad: useId(), errBad: useId(), note: useId() };
  const { status, record, points, report, error, maxPoints, sent, start } = useSweep();
  // the limits of the sweep on screen, so leaving the tab and coming back keeps them
  const [p99Text, setP99] = useState(() => String(sent?.p99_ms ?? 1000));
  const [errText, setErr] = useState(() => String(sent?.error_pct ?? 1));
  const live = isLive(useMode((m) => m.mode));
  const view = useDraft((s) => s.view);
  const runBusy = useRun((s) => s.status === "running");

  const p99Bad = p99Problem(p99Text);
  const errBad = errProblem(errText);
  // the server sweeps the applied design and picks its first profile; only a clean draft is it
  const noProfile = view !== null && !view.dirty && draftBody(view) === null;
  const running = status === "running";
  const note = live
    ? "A sweep steps the load, which a live system cannot. Switch to Batch to run one."
    : view === null
      ? "Loading the design; Start is ready once it arrives."
      : noProfile
        ? "The design has no traffic profile to sweep."
        : runBusy && !running
          ? "A run is in progress; start when it ends."
          : null;
  const canStart = note === null && p99Bad === null && errBad === null && !running;
  const why = [note && ids.note, p99Bad && ids.p99Bad, errBad && ids.errBad].filter(Boolean);

  const result = status === "done" ? record?.sweep : null;
  // a record is current only for the very generation of the very server process it was made on;
  // a sweep sent with no boot known cannot be called a restart
  const restarted =
    record !== null && view !== null && sent?.boot != null && sent.boot !== view.boot;
  const earlier =
    record !== null && view !== null && !restarted && record.generation !== view.generation;
  const stale = restarted || earlier;
  const limitMs = result?.limit.p99_ms ?? sent?.p99_ms ?? 1000;

  return (
    <Group id={RESULTS_GROUP.sweep} title="Find where it breaks">
      <div className="mb-2 flex flex-wrap items-end gap-3 text-xs">
        <label htmlFor={ids.p99} className="flex flex-col gap-0.5">
          p99 limit (ms)
          <input
            id={ids.p99}
            type="number"
            min={0}
            step="any"
            value={p99Text}
            aria-invalid={p99Bad !== null}
            aria-describedby={p99Bad ? ids.p99Bad : undefined}
            onChange={(e) => setP99(e.currentTarget.value)}
            className={input}
          />
        </label>
        <label htmlFor={ids.err} className="flex flex-col gap-0.5">
          error limit (%)
          <input
            id={ids.err}
            type="number"
            min={0}
            max={100}
            step="any"
            value={errText}
            aria-invalid={errBad !== null}
            aria-describedby={errBad ? ids.errBad : undefined}
            onChange={(e) => setErr(e.currentTarget.value)}
            className={input}
          />
        </label>
        <button
          type="button"
          disabled={!canStart}
          aria-describedby={why.length > 0 ? why.join(" ") : undefined}
          onClick={() => {
            if (!canStart) return;
            // no profile: the server sweeps the applied design's own first one
            void start({ p99_ms: Number(p99Text), error_pct: Number(errText) }, view?.boot ?? null);
          }}
          className="min-h-7 rounded border border-control bg-sheet px-3 hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-50"
        >
          Start
        </button>
      </div>
      {p99Bad ? (
        <p id={ids.p99Bad} className="mb-2 text-xs text-negative">
          {p99Bad}
        </p>
      ) : null}
      {errBad ? (
        <p id={ids.errBad} className="mb-2 text-xs text-negative">
          {errBad}
        </p>
      ) : null}
      {note ? (
        <p id={ids.note} className="mb-2 text-xs text-muted">
          {note}
        </p>
      ) : null}
      {!live && view?.dirty ? (
        <p className="mb-2 text-xs text-muted">
          Sweeps the applied design; Apply to include your edits.
        </p>
      ) : null}

      {/* the slot is taken: a state to wait out, not a failure */}
      {status === "busy" ? (
        <p className="mb-2 text-xs text-muted">
          The simulator is busy with another run or sweep. Start again when it ends.
        </p>
      ) : null}
      {running ? (
        <output className="mb-2 block text-xs text-muted">
          {`Sweeping: point ${points.length} of up to ${maxPoints}`}
        </output>
      ) : null}
      {status === "failed" ? (
        <p role="alert" className="mb-2 text-xs text-negative">
          {`The sweep failed: ${error ?? "no reason given"}`}
        </p>
      ) : null}

      {stale ? (
        <p className="mb-2 text-xs text-warn">
          {restarted
            ? "Made before the server restarted. Start a new sweep to see the design in force."
            : "Made on an earlier design: the design was applied again since. Start a new sweep to see the design in force."}
        </p>
      ) : null}
      {result ? <p className="mb-2 text-xs">{describeSweep(result)}</p> : null}
      {points.length > 0 && (running || status === "failed" || result) ? (
        <div className={stale ? "opacity-60" : undefined}>
          <LazyBoundary fallback={<div className="h-40" />}>
            <SweepChart
              points={points}
              limitMs={limitMs}
              knee={result?.verdict ?? null}
              firstToSaturate={result?.first_to_saturate ?? null}
            />
          </LazyBoundary>
        </div>
      ) : null}
      {result ? (
        <>
          <dl className="my-2 grid grid-cols-[auto_1fr] gap-x-2 text-xs">
            <dt className="text-muted">First to saturate</dt>
            <dd>
              {result.first_to_saturate
                ? `${result.first_to_saturate.node} at ${twoSignificant(result.first_to_saturate.req_s)} req/s${
                    result.first_to_saturate.analytic_req_s !== null
                      ? ` (queueing estimate ${twoSignificant(result.first_to_saturate.analytic_req_s)} req/s)`
                      : ""
                  }`
                : "none in the range swept"}
            </dd>
            <dt className="text-muted">Dominant cause</dt>
            <dd>{result.dominant_cause ?? "none"}</dd>
          </dl>
          {result.warnings.length > 0 ? (
            <ul className="mb-2 space-y-1 rounded-lg border border-warn p-2 text-xs text-warn">
              {result.warnings.map((w, i) => (
                <li key={i}>{w}</li>
              ))}
            </ul>
          ) : null}
          {report !== null && record ? (
            <CopySaveButtons text={() => report} fileName={`kaanal-sweep-${record.id}.md`} />
          ) : (
            <p className="mb-2 text-xs text-muted">{`The report could not be fetched: ${error ?? "no reason given"}`}</p>
          )}
        </>
      ) : null}
    </Group>
  );
}
