// A `call:` step's own retry (Part 9, S4). It replaces the wire's retry for this call, so the step either
// uses the wire's (nothing written) or carries its own `retry:` written with the same boxes, defaults and
// limits as a wire's (`RetryBody`). A step written as a bare name becomes a map when it gets one.

import { HelpPopover } from "../../overlays/help/HelpPopover";
import { useDraft } from "../../data/store/draft";
import { RetryBody } from "../details/ResilienceFields";
import { setFieldOps, type CallStep } from "./model";

const BTN =
  "mb-2 min-h-6 rounded-md border border-control px-2 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent";

const own = (v: unknown): Record<string, unknown> | null =>
  v !== null && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : null;

export function StepRetry({ step }: { step: CallStep }) {
  const patch = useDraft((s) => s.patch);
  const r = own(step.retry);
  return (
    <div className="mt-2 border-t pt-2">
      <p className="mb-1 flex items-center gap-0.5 text-xs font-medium">
        Retry on this call
        <HelpPopover id="step-retry" label="Retry on this call" />
      </p>
      {r === null ? (
        <>
          <p className="mb-2 text-xs text-muted">Uses the wire&apos;s retry, if it has one.</p>
          <button
            type="button"
            className={BTN}
            onClick={() => void patch(setFieldOps(step, "retry", {}))}
          >
            Give this call its own retry
          </button>
        </>
      ) : (
        <>
          <p className="mb-2 text-xs text-muted">
            This step&apos;s retry, instead of the wire&apos;s.
          </p>
          <RetryBody at={step.path} r={r} />
          <button
            type="button"
            className={BTN}
            onClick={() => void patch(setFieldOps(step, "retry", undefined))}
          >
            Use the wire&apos;s retry
          </button>
        </>
      )}
    </div>
  );
}
