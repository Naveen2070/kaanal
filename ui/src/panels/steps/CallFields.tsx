// The boxes of one expanded `call:` step: which component, which operation, how many times, how often,
// the body sizes and streaming. Empty means "as the wire says", and a Remove button puts the box back
// there. Each edit is one patch (`model.ts`); a step written as a bare name becomes a map on its first.

import { useDraft } from "../../data/store/draft";
import { NumberField } from "../details/Field";
import { BYTE_SIZE_UNITS, parseBytes } from "../details/units";
import { StepRetry } from "./StepRetry";
import { setCalleeOps, setFieldOps, type CallField, type CallStep } from "./model";

const OPS = ["read", "write", "scan"] as const;
const STREAMS = [
  ["", "As the wire says"],
  ["none", "Neither streams"],
  ["response", "The response streams"],
  ["request", "The request streams"],
  ["both", "Both stream"],
] as const;
const BODY_MAX_BYTES = 1024 ** 3;

const SELECT =
  "mt-0.5 w-full rounded-md border border-control bg-sheet px-1.5 py-1 text-xs focus-visible:outline-2 focus-visible:outline-accent";
const CLEAR =
  "mb-2 min-h-6 rounded-md border border-control px-2 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent";

export function CallFields({ step, callable }: { step: CallStep; callable: string[] }) {
  const patch = useDraft((s) => s.patch);
  const set = (field: CallField, value: unknown) => void patch(setFieldOps(step, field, value));
  const nodes = callable.includes(step.node) ? callable : [step.node, ...callable];
  const stream = typeof step.stream === "string" ? step.stream : "";
  return (
    <div className="px-2 pt-1 pb-2">
      <label className="mb-2 block text-xs">
        Component to call
        <select
          value={step.node}
          onChange={(e) => void patch(setCalleeOps(step, e.target.value))}
          className={`${SELECT} font-mono`}
        >
          {nodes.map((n) => (
            <option key={n} value={n}>
              {n}
            </option>
          ))}
        </select>
      </label>
      <label className="mb-2 block text-xs">
        Operation
        <select
          value={step.op ?? ""}
          onChange={(e) => set("op", e.target.value || undefined)}
          className={SELECT}
        >
          <option value="">From the route</option>
          {OPS.map((o) => (
            <option key={o} value={o}>
              {o}
            </option>
          ))}
        </select>
      </label>
      <NumberField
        label="Repeat"
        help="step-repeat"
        quietHint
        hint="How many calls in a row this step makes. Empty uses the wire's own calls per request."
        value={step.repeat ?? null}
        min={1}
        max={1000}
        integer
        placeholder="Wire's"
        onCommit={(n) => set("repeat", n)}
      />
      {step.repeat !== undefined ? (
        <button type="button" className={CLEAR} onClick={() => set("repeat", undefined)}>
          Remove the repeat
        </button>
      ) : null}
      <NumberField
        label="Chance (%)"
        help="step-chance"
        quietHint
        hint="The share of requests that make this call. Empty uses the wire's own chance."
        value={step.prob === undefined ? null : Math.round(step.prob * 1000) / 10}
        min={0.1}
        max={100}
        step="any"
        placeholder="Wire's"
        onCommit={(n) => set("prob", Math.round(n * 10) / 1000)}
      />
      {step.prob !== undefined ? (
        <button type="button" className={CLEAR} onClick={() => set("prob", undefined)}>
          Remove the chance
        </button>
      ) : null}
      {(["sends", "returns"] as const).map((field) => (
        <div key={field}>
          <NumberField
            label={field === "sends" ? "Request size" : "Response size"}
            help={field === "sends" ? "request-size" : "response-size"}
            quietHint
            hint={
              field === "sends"
                ? "The body this call carries. Empty uses the wire's request size."
                : "The body this call brings back. Empty uses the wire's response size."
            }
            value={step[field] === undefined ? null : parseBytes(step[field])}
            units={BYTE_SIZE_UNITS}
            placeholder="Wire's"
            max={BODY_MAX_BYTES}
            step="any"
            onCommit={(n) => set(field, Math.round(n))}
          />
          {step[field] !== undefined ? (
            <button type="button" className={CLEAR} onClick={() => set(field, undefined)}>
              {field === "sends" ? "Remove the request size" : "Remove the response size"}
            </button>
          ) : null}
        </div>
      ))}
      <label className="block text-xs">
        Streaming
        <select
          value={stream}
          onChange={(e) => set("stream", e.target.value || undefined)}
          className={SELECT}
        >
          {STREAMS.map(([v, label]) => (
            <option key={v} value={v}>
              {label}
            </option>
          ))}
        </select>
      </label>
      <StepRetry step={step} />
      {step.kept.length > 0 ? (
        <p className="mt-2 mb-0 text-[11px] text-muted">
          Also written on this step and kept as it is: {step.kept.join(", ")}.
        </p>
      ) : null}
    </div>
  );
}
