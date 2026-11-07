// An expanded `cache:` step: which cache node it looks in. A hit ends the sequence the step sits in, so
// the steps after it only run on a miss.

import { useDraft } from "../../data/store/draft";
import { setCacheOps, type CacheStep } from "./model";

export function CacheFields({ step, caches }: { step: CacheStep; caches: string[] }) {
  const patch = useDraft((s) => s.patch);
  const nodes = caches.includes(step.node) ? caches : [step.node, ...caches];
  return (
    <div className="px-2 pt-1 pb-2">
      <label className="mb-2 block text-xs">
        Cache to look up
        <select
          value={step.node}
          onChange={(e) => void patch(setCacheOps(step, e.target.value))}
          className="mt-0.5 w-full rounded-md border border-control bg-sheet px-1.5 py-1 font-mono text-xs focus-visible:outline-2 focus-visible:outline-accent"
        >
          {nodes.map((n) => (
            <option key={n} value={n}>
              {n}
            </option>
          ))}
        </select>
      </label>
      <p className="m-0 text-[11px] text-muted">
        A hit ends the sequence this step is in: the steps after it only run on a miss.
      </p>
    </div>
  );
}
