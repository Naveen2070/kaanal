// The "machine" boxes of a component's CPUs group (steps 8P and 8M): a cloud instance type that
// fills the cores, the network card and the memory you leave out, and a memory limit that caps how
// many requests the copy holds at once. Both are patches of the draft (`fields.ts`).

import { useEffect, useId, useState } from "react";
import type { NodeDoc } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { FieldLabel, NumberField } from "./Field";
import {
  customInstancesOf,
  gcOf,
  instanceOf,
  memoryOf,
  onLimitOf,
  restartMsOf,
  setGc,
  setMemory,
  setOnLimit,
  setRestart,
  type OnLimit,
} from "./fields";
import { findInstance, loadInstances, type InstanceRow } from "../../data/api/instances";
import { BYTE_SIZE_UNITS, TIME_UNITS } from "./units";

const GIB = 1024 ** 3;

/** A hard memory limit: how much the copy has, what one request holds, what it holds idle. */
export function MemoryFields({ name, config }: { name: string; config: NodeDoc }) {
  const patch = useDraft((s) => s.patch);
  const doc = useDraft((s) => s.view?.doc ?? null);
  const [rows, setRows] = useState<InstanceRow[] | null>(null);
  const m = memoryOf(config);
  const inst = instanceOf(config);
  // only a cloud type needs the table: one of the file's own is read from the draft
  const cloud = inst !== null && !(inst in customInstancesOf(doc));
  useEffect(() => {
    if (!cloud) return;
    let gone = false;
    loadInstances().then(
      (r) => !gone && setRows(r),
      () => undefined,
    );
    return () => {
      gone = true;
    };
  }, [cloud]);
  // the memory the node's instance gives: one of the file's own, or a type of the cloud table
  const fromInstance: number | null =
    inst === null
      ? null
      : (customInstancesOf(doc)[inst]?.memory ??
          (rows ? (findInstance(rows, inst)?.memMib ?? 0) * 1024 ** 2 : 0)) ||
        null;
  const slots =
    m.per_request !== null && (m.limit !== null || fromInstance !== null)
      ? Math.floor(((m.limit ?? fromInstance ?? 0) - (m.base ?? 0)) / m.per_request)
      : null;
  const box = (
    key: "limit" | "per_request" | "base",
    label: string,
    hint: string,
    placeholder: string,
  ) => (
    <NumberField
      label={label}
      hint={hint}
      value={m[key]}
      units={BYTE_SIZE_UNITS}
      placeholder={placeholder}
      min={1}
      step="any"
      onCommit={(n) => patch(setMemory(name, config, key, n))}
    />
  );
  return (
    <>
      {box(
        "limit",
        "Memory",
        "What each copy has. With a size per request below, the copy holds only as many requests at once as fit; the rest wait in the queue or are refused. No garbage-collection or swap slowdown is modelled.",
        fromInstance !== null
          ? `${Number((fromInstance / GIB).toFixed(2))} GiB (instance)`
          : "Not limited",
      )}
      {box(
        "per_request",
        "Memory per request",
        "What one request being served holds. Required for a limit to mean anything.",
        "Not set",
      )}
      {box("base", "Memory when idle", "What the process holds with no request. Default 0.", "0")}
      {"memory" in config ? (
        <NumberField
          label="Garbage collection cost"
          hint="Off when empty. What collecting costs per byte allocated, against the work itself (0.2 is a fifth): own work then takes 1 + cost × u / (1 − u) times its time, u being the live share of the memory. An estimate: the shape is published, the weight depends on your runtime."
          value={gcOf(config)}
          placeholder="Not modelled"
          min={0.001}
          max={10}
          step="any"
          onCommit={(n) => patch(setGc(name, config, n))}
        />
      ) : null}
      {"memory" in config ? <OnLimitFields name={name} config={config} /> : null}
      {slots !== null ? (
        <p className="mb-2 text-xs text-muted">
          {slots >= 1
            ? `The memory holds ${slots.toLocaleString("en-US")} request${slots === 1 ? "" : "s"} at once per copy.`
            : "The memory cannot hold even one request: the file will not load."}
        </p>
      ) : null}
    </>
  );
}

/** What a copy does when its memory is full: turn the request away, or die and restart (step 8K). */
function OnLimitFields({ name, config }: { name: string; config: NodeDoc }) {
  const patch = useDraft((s) => s.patch);
  const id = useId();
  const mode = onLimitOf(config);
  return (
    <>
      <div className="mb-2">
        <FieldLabel htmlFor={id} label="When memory runs out" />
        <select
          id={id}
          value={mode}
          aria-describedby={`${id}-hint`}
          onChange={(e) => void patch(setOnLimit(name, config, e.currentTarget.value as OnLimit))}
          className="h-7 rounded border border-control bg-sheet px-1 text-text"
        >
          <option value="refuse">Turn the request away</option>
          <option value="kill">Kill the copy</option>
        </select>
        <p id={`${id}-hint`} className="mt-1 text-xs text-muted">
          {mode === "kill"
            ? "A request the memory cannot hold kills the whole copy, as the OOM killer would: what it is serving or has queued fails with a 502, and it refuses calls (503) until it restarts. Other copies carry on."
            : "A request the memory cannot hold waits in the queue, or is refused (503)."}
        </p>
      </div>
      {mode === "kill" ? (
        <NumberField
          label="Restart after"
          hint="How long the killed copy stays down before it is back, empty."
          value={restartMsOf(config)}
          units={TIME_UNITS}
          min={1}
          max={3_600_000}
          step="any"
          onCommit={(n) => patch(setRestart(name, config, n))}
        />
      ) : null}
    </>
  );
}
