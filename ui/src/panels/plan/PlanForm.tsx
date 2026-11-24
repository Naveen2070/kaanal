// The plan's form (step 9c-3): the target, the two limits, an optional budget, the nodes to keep as
// they are, and the cheapest search with its time limit, run cap and providers. The values live in the
// plan store (the dialog unmounts when closed); toBody checks them against the server's own bounds, so
// each invalid field says why before a request is sent.

import { useId } from "react";
import type { PlanBody } from "../../data/api/types.plan";
import { useDraft } from "../../data/store/draft";
import { clean, toBody, type FormState, type Problems, type Provider } from "./planMath";

const input = "h-7 w-24 rounded border border-control bg-sheet text-text px-1";
const label = "flex flex-col gap-0.5";
const PROVIDERS: [Provider, string][] = [
  ["aws", "AWS"],
  ["gcp", "GCP"],
  ["azure", "Azure"],
];

type Field = keyof Problems;

export function PlanForm({
  value,
  onChange,
  disabledWhy,
  running = false,
  onStart,
}: {
  value: FormState;
  /** Gets the latest form (the store's, not this render's) and gives back the next: two edits in a row never lose the first. */
  onChange: (next: (cur: FormState) => FormState) => void;
  /** Why Start is off (a note shown by the dialog), as the id of that note; null when it is on. */
  disabledWhy: string | null;
  running?: boolean;
  onStart: (body: PlanBody) => void;
}) {
  const base = useId();
  const id = (k: string) => `${base}-${k}`;
  const names = Object.keys(useDraft((s) => s.view?.doc?.nodes) ?? {});
  // a ticked name that is no longer a node of the draft is dropped, not sent (the server would refuse it)
  // with no readable draft there is no list to check against: send what was ticked
  const keep = names.length > 0 ? value.keep.filter((k) => names.includes(k)) : value.keep;
  const dirty = useDraft((s) => s.view?.dirty ?? false);
  const out = toBody({ ...value, keep });
  const problems: Problems = "problems" in out ? out.problems : {};
  const set = (over: Partial<FormState>) => onChange((cur) => ({ ...cur, ...over }));
  const why = [disabledWhy, ...Object.keys(problems).map((k) => id(`${k}-bad`))].filter(
    (x): x is string => x !== null,
  );
  const canStart = "body" in out && disabledWhy === null && !running;

  const num = (k: Field, text: string, extra: { min?: number; max?: number; step?: string }) => ({
    id: id(k),
    type: "number",
    step: "any",
    ...extra,
    value: text,
    "aria-invalid": problems[k] !== undefined,
    "aria-describedby": problems[k] ? id(`${k}-bad`) : undefined,
    className: input,
  });

  const providers = value.providers;
  const toggle = (p: Provider) =>
    onChange((cur) => {
      const now = cur.providers === "none" ? [] : cur.providers;
      return { ...cur, providers: now.includes(p) ? now.filter((x) => x !== p) : [...now, p] };
    });

  return (
    <div className="text-xs">
      <div className="flex flex-wrap items-end gap-3">
        <label htmlFor={id("target")} className={label}>
          Target
          <input
            {...num("target", value.target, { min: 0 })}
            onChange={(e) => set({ target: e.currentTarget.value })}
          />
        </label>
        <label htmlFor={id("kind")} className={label}>
          Target unit
          <select
            id={id("kind")}
            value={value.targetKind}
            onChange={(e) => set({ targetKind: e.currentTarget.value as FormState["targetKind"] })}
            className="h-7 rounded border border-control bg-sheet px-1 text-text"
          >
            <option value="multiple">x of the profile's rates</option>
            <option value="req_s">req/s</option>
          </select>
        </label>
        <label htmlFor={id("p99")} className={label}>
          p99 limit (ms)
          <input
            {...num("p99", value.p99, { min: 0 })}
            onChange={(e) => set({ p99: e.currentTarget.value })}
          />
        </label>
        <label htmlFor={id("err")} className={label}>
          error limit (%)
          <input
            {...num("err", value.err, { min: 0, max: 100 })}
            onChange={(e) => set({ err: e.currentTarget.value })}
          />
        </label>
        <label htmlFor={id("budget")} className={label}>
          Budget ($/h, optional)
          <input
            {...num("budget", value.budget, { min: 0 })}
            onChange={(e) => set({ budget: e.currentTarget.value })}
          />
        </label>
      </div>

      {names.length > 0 ? (
        <details className="mt-2">
          <summary className="cursor-pointer">
            {`Keep as they are (${keep.length} of ${names.length})`}
          </summary>
          {dirty ? (
            <p className="m-0 mt-1 pl-4 text-muted">
              The names are from your draft; the plan checks them against the applied design, and a
              name that is not in it is refused.
            </p>
          ) : null}
          <ul className="mt-1 flex flex-wrap gap-x-3 gap-y-1 pl-4">
            {names.map((n) => (
              <li key={n}>
                <label className="flex items-center gap-1 [overflow-wrap:anywhere]">
                  <input
                    type="checkbox"
                    checked={keep.includes(n)}
                    onChange={() =>
                      onChange((cur) => ({
                        ...cur,
                        keep: cur.keep.includes(n)
                          ? cur.keep.filter((k) => k !== n)
                          : [...cur.keep, n],
                      }))
                    }
                  />
                  {clean(n)}
                </label>
              </li>
            ))}
          </ul>
        </details>
      ) : null}

      <label className="mt-2 flex items-center gap-1">
        <input
          type="checkbox"
          checked={value.cheapest}
          onChange={() => onChange((cur) => ({ ...cur, cheapest: !cur.cheapest }))}
        />
        Also look for the cheapest design
      </label>
      {value.cheapest ? (
        <div className="mt-2 flex flex-wrap items-end gap-3 pl-4">
          <label htmlFor={id("time")} className={label}>
            Time limit (s)
            <input
              {...num("time", value.time, { min: 1, max: 600 })}
              onChange={(e) => set({ time: e.currentTarget.value })}
            />
          </label>
          <label htmlFor={id("runCap")} className={label}>
            Run cap
            <input
              {...num("runCap", value.runCap, { min: 1, max: 2000, step: "1" })}
              onChange={(e) => set({ runCap: e.currentTarget.value })}
            />
          </label>
          <fieldset className="m-0 flex flex-wrap items-center gap-3 border-0 p-0">
            <legend className="mb-0.5 p-0">
              Providers to compare (none ticked: the server picks)
            </legend>
            {PROVIDERS.map(([p, name]) => (
              <label key={p} className="flex items-center gap-1">
                <input
                  type="checkbox"
                  checked={providers !== "none" && providers.includes(p)}
                  onChange={() => toggle(p)}
                />
                {name}
              </label>
            ))}
            <label className="flex items-center gap-1">
              <input
                type="checkbox"
                checked={providers === "none"}
                onChange={() =>
                  onChange((cur) => ({ ...cur, providers: cur.providers === "none" ? [] : "none" }))
                }
              />
              None
            </label>
          </fieldset>
        </div>
      ) : null}

      {(Object.entries(problems) as [Field, string][]).map(([k, text]) => (
        <p key={k} id={id(`${k}-bad`)} className="mt-2 text-negative">
          {text}
        </p>
      ))}

      <button
        type="button"
        disabled={!canStart}
        aria-describedby={why.length > 0 ? why.join(" ") : undefined}
        onClick={() => {
          if (canStart) onStart(out.body);
        }}
        className="mt-3 min-h-7 rounded border border-control bg-sheet px-3 hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-50"
      >
        {running ? "Planning..." : "Start"}
      </button>
    </div>
  );
}
