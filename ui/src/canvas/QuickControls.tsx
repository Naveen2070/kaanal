// The small card under each node: the two settings people reach for most. Each edit is a patch of
// the draft (the auto re-run then follows). A setting the config has no field for is left out:
// error rate is accepted but not active yet (docs/web/nodes.md), so it is not offered.

import type { NodeDoc } from "../api/types";
import { durationMs, hasProc, LIMITS, setDuration, setReplicas } from "../panel/fields";
import { useDraft } from "../store/draft";
import { replicasOf } from "./metrics";

export function QuickControls({ name, config }: { name: string; config: NodeDoc }) {
  const patch = useDraft((s) => s.patch);
  const ms = durationMs(config.proc);
  // a node with no `proc` at all can take one; a distribution is edited in the panel instead
  const canEditTime = hasProc(config.type) && (ms !== null || !("proc" in config));
  const id = `qc-${encodeURIComponent(name)}`;

  const commitReplicas = (raw: string) => {
    const n = Math.trunc(Number(raw));
    if (!Number.isInteger(n) || n < LIMITS.instances.min || n > LIMITS.instances.max) return;
    if (n === replicasOf(config)) return;
    void patch([setReplicas(name, config, n)]);
  };
  const commitTime = (raw: string) => {
    const n = Number(raw);
    if (raw.trim() === "" || !Number.isFinite(n)) return;
    if (n < LIMITS.serviceMs.min || n > LIMITS.serviceMs.max || n === (ms ?? 0)) return;
    void patch([setDuration(name, config, "proc", n)]);
  };
  const commitOnEnter =
    (commit: (v: string) => void) => (e: React.KeyboardEvent<HTMLInputElement>) => {
      if (e.key === "Enter") commit(e.currentTarget.value);
    };

  return (
    // nodrag/nopan: typing in a field must not drag or pan the canvas
    <div className="nodrag nopan mt-1 flex gap-2 border-t pt-1 text-xs">
      <label htmlFor={`${id}-replicas`} className="flex items-center gap-1">
        Instances
        <input
          id={`${id}-replicas`}
          key={replicasOf(config)}
          type="number"
          min={1}
          max={64}
          defaultValue={replicasOf(config)}
          onBlur={(e) => commitReplicas(e.currentTarget.value)}
          onKeyDown={commitOnEnter(commitReplicas)}
          className="w-12 rounded border px-1"
        />
      </label>
      {canEditTime ? (
        <label htmlFor={`${id}-time`} className="flex items-center gap-1">
          Time (ms)
          <input
            id={`${id}-time`}
            key={ms ?? "none"}
            type="number"
            min={0}
            step="any"
            defaultValue={ms ?? ""}
            onBlur={(e) => commitTime(e.currentTarget.value)}
            onKeyDown={commitOnEnter(commitTime)}
            className="w-16 rounded border px-1"
          />
        </label>
      ) : null}
    </div>
  );
}
