// The Controls boxes for what a wire does when a call over it fails (step 7): retries and their
// budget, what happens to a call's work when the caller gives up, a circuit breaker, and how long
// a balancer takes to notice a downed copy. Each is off until switched on; every edit is a patch of
// the draft (see `resilience.ts`).

import { useId } from "react";
import { useDraft } from "../../data/store/draft";
import type { EdgeDoc, NodeDoc } from "../../data/api/types";
import { NumberField, Section } from "./Field";
import {
  ABANDON_CHOICES,
  abandonOf,
  type AbandonChoice,
  attemptsOf,
  backoffMsOf,
  BREAKER,
  breakerOf,
  budgetOf,
  budgetRatioOf,
  budgetTokensOf,
  capMsOf,
  clearDetectEdge,
  clearDetectNode,
  DETECT_MAX_MS,
  detectMsOf,
  JITTERS,
  jitterOf,
  type Jitter,
  minCallsOf,
  multiplierOf,
  openForMsOf,
  probesOf,
  ratePctOf,
  RETRY,
  retryOf,
  setAbandon,
  setBreaker,
  setBreakerOn,
  setBudget,
  setBudgetOn,
  setDetectEdge,
  setDetectNode,
  setJitter,
  setOpenFor,
  setRatePct,
  setRetry,
  setRetryMs,
  setRetryOn,
  setWrites,
  windowOf,
  writesOf,
} from "./resilience";
import { TIME_UNITS } from "./units";

const clearButton =
  "mb-2 min-h-6 rounded-md border border-control px-2 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent";

/** A checkbox with its hint under it: the way to switch a whole feature on or off. */
function Toggle({
  label,
  hint,
  on,
  onChange,
}: {
  label: string;
  hint: string;
  on: boolean;
  onChange: (on: boolean) => void;
}) {
  const id = useId();
  return (
    <div className="mb-2">
      <label htmlFor={id} className="flex items-center gap-1.5 text-sm font-medium">
        <input
          id={id}
          type="checkbox"
          checked={on}
          aria-describedby={`${id}-hint`}
          onChange={(ev) => onChange(ev.currentTarget.checked)}
        />
        {label}
      </label>
      <p id={`${id}-hint`} className="text-xs text-muted">
        {hint}
      </p>
    </div>
  );
}

/** Retries, the retry budget and what the caller's giving up does (RULES 82-84, 88). */
export function RetryFields({ index, e }: { index: number; e: EdgeDoc }) {
  const patch = useDraft((s) => s.patch);
  const jitterId = useId();
  const abandonId = useId();
  const r = retryOf(e);
  const budget = r ? budgetOf(r) : null;
  return (
    <Section title="When a call fails">
      <Toggle
        label="Retry a failed call"
        hint="A call that times out or gets a 5xx, 429 or reset is tried again after a wait. Off means one try."
        on={r !== null}
        onChange={(on) => void patch(setRetryOn(index, e, on))}
      />
      {r ? (
        <>
          <NumberField
            label="Tries (the first counts)"
            hint="Most tries a call makes, 2 to 5; with 3 a failed call is retried twice."
            value={attemptsOf(r)}
            min={RETRY.attempts.min}
            max={RETRY.attempts.max}
            integer
            onCommit={(n) => patch([setRetry(index, r, "max_attempts", n)])}
          />
          <NumberField
            label="First wait"
            help="time"
            hint="The wait before the first retry; each later wait is longer by the growth below."
            value={backoffMsOf(r) ?? RETRY.backoffMs.default}
            units={TIME_UNITS}
            min={RETRY.backoffMs.min}
            max={RETRY.backoffMs.max}
            step="any"
            onCommit={(n) => patch([setRetryMs(index, r, "backoff", n)])}
          />
          <NumberField
            label="Growth of the wait"
            hint="Each wait is this many times the one before (2 doubles it, 1 keeps it constant)."
            value={multiplierOf(r)}
            min={RETRY.multiplier.min}
            max={RETRY.multiplier.max}
            integer
            onCommit={(n) => patch([setRetry(index, r, "multiplier", n)])}
          />
          <NumberField
            label="Longest wait"
            help="time"
            hint="A wait never grows past this."
            value={capMsOf(r) ?? RETRY.capMs.default}
            units={TIME_UNITS}
            min={RETRY.capMs.min}
            max={RETRY.capMs.max}
            step="any"
            onCommit={(n) => patch([setRetryMs(index, r, "cap", n)])}
          />
          <div className="mb-2">
            <label htmlFor={jitterId} className="block text-sm font-medium">
              Randomise the wait
            </label>
            <select
              id={jitterId}
              value={jitterOf(r)}
              aria-describedby={`${jitterId}-hint`}
              onChange={(ev) => void patch(setJitter(index, r, ev.currentTarget.value as Jitter))}
              className="h-7 rounded border border-control bg-sheet px-1 text-text"
            >
              {JITTERS.map((j) => (
                <option key={j.value} value={j.value}>
                  {j.label}
                </option>
              ))}
            </select>
            <p id={`${jitterId}-hint`} className="text-xs text-muted">
              Spreading the waits keeps many callers from retrying in the same instant.
            </p>
          </div>
          <Toggle
            label="Retry writes too"
            hint="A retried write to a database or service can apply twice, so it is off by default. A write to a file store always retries (it is safe to repeat)."
            on={writesOf(r)}
            onChange={(on) => void patch(setWrites(index, r, on))}
          />
          <Toggle
            label="Limit retries with a budget"
            hint="gRPC's retry throttling: each failed call spends a token, each success gives a fraction back, and no retry is made while half the tokens or fewer are left. It stops a retry storm."
            on={budget !== null}
            onChange={(on) => void patch(setBudgetOn(index, r, on))}
          />
          {budget ? (
            <>
              <NumberField
                label="Budget (tokens)"
                hint="Size of the budget, 1 to 1000."
                value={budgetTokensOf(budget)}
                min={RETRY.budgetTokens.min}
                max={RETRY.budgetTokens.max}
                integer
                onCommit={(n) => patch([setBudget(index, budget, "max_tokens", n)])}
              />
              <NumberField
                label="Refill per success"
                hint="Tokens a success gives back, above 0 and at most 1 (0.1 means ten successes pay for one retry)."
                value={budgetRatioOf(budget)}
                min={RETRY.budgetRatio.min}
                max={RETRY.budgetRatio.max}
                step="any"
                onCommit={(n) => patch([setBudget(index, budget, "token_ratio", n)])}
              />
            </>
          ) : null}
        </>
      ) : null}
      <div className="mb-2">
        <label htmlFor={abandonId} className="block text-sm font-medium">
          When the caller gives up
        </label>
        <select
          id={abandonId}
          value={abandonOf(e)}
          aria-describedby={`${abandonId}-hint`}
          onChange={(ev) =>
            void patch(setAbandon(index, e, ev.currentTarget.value as AbandonChoice))
          }
          className="h-7 rounded border border-control bg-sheet px-1 text-text"
        >
          {ABANDON_CHOICES.map((c) => (
            <option key={c.value} value={c.value}>
              {c.label}
            </option>
          ))}
        </select>
        <p id={`${abandonId}-hint`} className="text-xs text-muted">
          After a timeout, most servers do not notice the caller left and finish the work anyway.
          Let the callee finish it to see that load: with retries it is what feeds a retry storm.
        </p>
      </div>
    </Section>
  );
}

/** The circuit breaker on a wire (RULES 90). */
export function BreakerFields({ index, e }: { index: number; e: EdgeDoc }) {
  const patch = useDraft((s) => s.patch);
  const b = breakerOf(e);
  return (
    <Section title="Circuit breaker">
      <Toggle
        label="Stop calling after repeated failures"
        hint="When enough of the last calls failed, the caller stops calling for a while and answers 503 at once, then lets a few calls through to see whether the callee is back."
        on={b !== null}
        onChange={(on) => void patch(setBreakerOn(index, e, on))}
      />
      {b ? (
        <>
          <NumberField
            label="Calls remembered"
            hint="How many of the latest calls the breaker looks at, 1 to 1000."
            value={windowOf(b)}
            min={BREAKER.window.min}
            max={BREAKER.window.max}
            integer
            onCommit={(n) => patch([setBreaker(index, b, "window", n)])}
          />
          <NumberField
            label="Calls needed before it can open"
            hint="It never opens on fewer calls than this (it counts as the number remembered when larger)."
            value={minCallsOf(b)}
            min={BREAKER.minCalls.min}
            max={BREAKER.minCalls.max}
            integer
            onCommit={(n) => patch([setBreaker(index, b, "min_calls", n)])}
          />
          <NumberField
            label="Open at a failure rate of (%)"
            hint="It opens when at least this share of the remembered calls failed (a 5xx, a timeout or a reset; a 4xx does not count)."
            value={ratePctOf(b)}
            min={BREAKER.ratePct.min}
            max={BREAKER.ratePct.max}
            step="any"
            onCommit={(n) => patch([setRatePct(index, b, n)])}
          />
          <NumberField
            label="Stays open for"
            help="time"
            hint="How long it refuses every call before it tries again."
            value={openForMsOf(b) ?? BREAKER.openForMs.default}
            units={TIME_UNITS}
            min={BREAKER.openForMs.min}
            max={BREAKER.openForMs.max}
            step="any"
            onCommit={(n) => patch([setOpenFor(index, b, n)])}
          />
          <NumberField
            label="Trial calls"
            hint="How many calls it lets through to decide: it closes when fewer than the failure rate fail, and opens again otherwise."
            value={probesOf(b)}
            min={BREAKER.probes.min}
            max={BREAKER.probes.max}
            integer
            onCommit={(n) => patch([setBreaker(index, b, "half_open_calls", n)])}
          />
        </>
      ) : null}
    </Section>
  );
}

const DETECT_HINT =
  "How long calls keep going to a copy that has gone down before the balancer notices (the health-check interval times its unhealthy threshold). A copy that is down when the run starts is already noticed. Empty means at once.";

/** `detect` on a wire to a component with copies (RULES 91). */
export function EdgeDetectField({ index, e }: { index: number; e: EdgeDoc }) {
  const patch = useDraft((s) => s.patch);
  const ms = detectMsOf(e);
  return (
    <>
      <NumberField
        label="Notice a downed copy after"
        help="time"
        hint={DETECT_HINT}
        value={ms}
        units={TIME_UNITS}
        placeholder="At once"
        min={0}
        max={DETECT_MAX_MS}
        step="any"
        onCommit={(n) => patch([setDetectEdge(index, e, n)])}
      />
      {"detect" in e ? (
        <button
          type="button"
          onClick={() => void patch(clearDetectEdge(index, e))}
          className={clearButton}
        >
          Back to the default
        </button>
      ) : null}
    </>
  );
}

/** `detect` on a load-balancer node: the delay for the pool behind it. */
export function NodeDetectField({ name, node }: { name: string; node: NodeDoc }) {
  const patch = useDraft((s) => s.patch);
  return (
    <>
      <NumberField
        label="Notice a downed copy after"
        help="time"
        hint={`${DETECT_HINT} A wire's own setting wins.`}
        value={detectMsOf(node)}
        units={TIME_UNITS}
        placeholder="At once"
        min={0}
        max={DETECT_MAX_MS}
        step="any"
        onCommit={(n) => patch([setDetectNode(name, node, n)])}
      />
      {"detect" in node ? (
        <button
          type="button"
          onClick={() => void patch(clearDetectNode(name, node))}
          className={clearButton}
        >
          Back to the default
        </button>
      ) : null}
    </>
  );
}
