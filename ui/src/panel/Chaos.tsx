// Break things on purpose: take a node, one instance of it, or a link down, slow it, or drop some
// of its requests. Targets are structured objects (`{node, replica}`, `{edge: {from, to}}`), never
// "a->b" strings. Chaos is live: the next run sees it.

import { useEffect, useId, useState } from "react";
import { ApiError, api } from "../api/client";
import type { Topology } from "../api/types";
import { useDraft } from "../store/draft";
import { chaosBody, type ChaosAction, type ChaosTarget } from "./fields";

export function Chaos({
  name,
  replicas,
  links,
  onApplied,
}: {
  name: string;
  replicas: number;
  /** Names this node calls. */
  links: string[];
  /** Chaos was set: the numbers on screen no longer describe it. */
  onApplied: () => void;
}) {
  const id = useId();
  const generation = useDraft((s) => s.view?.generation);
  /** The applied system: undefined while loading, null when there is none (a blank start). */
  const [applied, setApplied] = useState<Topology | null | undefined>(undefined);
  useEffect(() => {
    let live = true;
    api
      .getTopology()
      .then((t) => live && setApplied(t))
      .catch(() => live && setApplied(null));
    return () => {
      live = false;
    };
  }, [name, generation]);
  // chaos acts on the system in force, but the canvas draws the draft: a node or link that is only
  // in the draft has nothing to break yet
  const nodeKnown = applied?.nodes.some((n) => n.name === name) ?? false;
  const linkKnown = (to: string) =>
    applied?.edges.some((e) => e.from === name && e.to === to) ?? false;
  const [target, setTarget] = useState("node");
  const [ms, setMs] = useState("100");
  const [percent, setPercent] = useState("10");
  const [msg, setMsg] = useState<string | null>(null);

  const resolve = (): ChaosTarget => {
    if (target.startsWith("replica:")) return { node: name, replica: Number(target.slice(8)) };
    if (target.startsWith("link:")) return { edge: { from: name, to: target.slice(5) } };
    return { node: name };
  };
  const run = async (action: ChaosAction) => {
    // an empty or non-numeric box is not a value: nothing is sent
    if (action.kind === "slow" && !(Number.isFinite(action.ms) && action.ms >= 0)) return;
    if (
      action.kind === "drop" &&
      !(Number.isFinite(action.rate) && action.rate >= 0 && action.rate <= 1)
    )
      return;
    try {
      await api.setChaos(chaosBody(resolve(), action));
      setMsg(null);
      onApplied();
    } catch (e) {
      setMsg(e instanceof ApiError ? e.message : String(e));
    }
  };

  return (
    <div>
      <label htmlFor={`${id}-t`} className="block text-sm font-medium">
        Break
      </label>
      <select
        id={`${id}-t`}
        value={target}
        onChange={(e) => setTarget(e.currentTarget.value)}
        className="mb-1 rounded border px-1 text-sm"
      >
        <option value="node">the whole node</option>
        {Array.from({ length: replicas > 1 ? replicas : 0 }, (_, i) => (
          <option key={`r${i}`} value={`replica:${i}`}>
            instance {i}
          </option>
        ))}
        {links.map((to) => (
          <option key={`l${to}`} value={`link:${to}`} disabled={!linkKnown(to)}>
            the link to {to}
          </option>
        ))}
      </select>
      <div className="flex flex-wrap items-center gap-1 text-sm">
        <button
          type="button"
          className="rounded border px-2 disabled:opacity-40"
          disabled={!nodeKnown}
          aria-describedby={nodeKnown ? undefined : `${id}-why`}
          onClick={() => void run({ kind: "down", down: true })}
        >
          Take down
        </button>
        <button
          type="button"
          className="rounded border px-2 disabled:opacity-40"
          disabled={!nodeKnown}
          aria-describedby={nodeKnown ? undefined : `${id}-why`}
          onClick={() => void run({ kind: "down", down: false })}
        >
          Bring back
        </button>
        <button
          type="button"
          className="rounded border px-2 disabled:opacity-40"
          disabled={!nodeKnown}
          aria-describedby={nodeKnown ? undefined : `${id}-why`}
          onClick={() => void run({ kind: "clear" })}
        >
          Clear
        </button>
      </div>
      <div className="mt-1 flex items-center gap-1 text-sm">
        <label htmlFor={`${id}-ms`}>Slow by (ms)</label>
        <input
          id={`${id}-ms`}
          type="number"
          min={0}
          max={300000}
          value={ms}
          onChange={(e) => setMs(e.currentTarget.value)}
          className="w-20 rounded border px-1"
        />
        <button
          type="button"
          className="rounded border px-2 disabled:opacity-40"
          disabled={!nodeKnown}
          aria-describedby={nodeKnown ? undefined : `${id}-why`}
          onClick={() =>
            void run({ kind: "slow", ms: ms.trim() === "" ? NaN : Math.trunc(Number(ms)) })
          }
        >
          Apply
        </button>
      </div>
      <div className="mt-1 flex items-center gap-1 text-sm">
        <label htmlFor={`${id}-p`}>Drop (%)</label>
        <input
          id={`${id}-p`}
          type="number"
          min={0}
          max={100}
          value={percent}
          onChange={(e) => setPercent(e.currentTarget.value)}
          className="w-20 rounded border px-1"
        />
        <button
          type="button"
          className="rounded border px-2 disabled:opacity-40"
          disabled={!nodeKnown}
          aria-describedby={nodeKnown ? undefined : `${id}-why`}
          onClick={() =>
            void run({ kind: "drop", rate: percent.trim() === "" ? NaN : Number(percent) / 100 })
          }
        >
          Apply
        </button>
      </div>
      <p id={`${id}-why`} className="mt-1 text-xs text-muted">
        {nodeKnown
          ? "Live chaos, not part of the file. A run follows."
          : applied === undefined
            ? "Checking the running system..."
            : "Chaos acts on the running system, and this component is not in it yet (it is only in the draft). Apply the draft first."}
      </p>
      {msg ? (
        <p role="alert" className="text-xs text-red-700">
          {msg}
        </p>
      ) : null}
    </div>
  );
}
