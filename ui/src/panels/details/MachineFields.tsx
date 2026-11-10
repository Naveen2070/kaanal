// The "machine" boxes of a component's CPUs group (steps 8P and 8M): a cloud instance type that
// fills the cores, the network card and the memory you leave out, and a memory limit that caps how
// many requests the copy holds at once. Both are patches of the draft (`fields.ts`).

import { useId } from "react";
import type { NodeDoc } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { FieldLabel, NumberField } from "./Field";
import { instanceOf, memoryOf, setInstance, setMemory } from "./fields";
import { INSTANCE_TYPES, instanceByName, instanceLine } from "./instances";
import { BYTE_SIZE_UNITS } from "./units";

const GIB = 1024 ** 3;

/** Picks a cloud instance type; its cores, card and memory are used wherever the box is empty. */
export function InstanceField({ name, config }: { name: string; config: NodeDoc }) {
  const patch = useDraft((s) => s.patch);
  const id = useId();
  const now = instanceOf(config);
  const known = now !== null ? instanceByName(now) : undefined;
  return (
    <div className="mb-2">
      <FieldLabel htmlFor={id} label="Instance type" />
      <select
        id={id}
        value={now ?? ""}
        aria-describedby={`${id}-hint`}
        onChange={(e) => void patch(setInstance(name, config, e.currentTarget.value || null))}
        className="h-7 max-w-full rounded border border-control bg-sheet px-1 text-text"
      >
        <option value="">None</option>
        {now !== null && known === undefined ? <option value={now}>{now}</option> : null}
        {INSTANCE_TYPES.map((i) => (
          <option key={i.name} value={i.name}>
            {i.name}
          </option>
        ))}
      </select>
      <p id={`${id}-hint`} className="mt-1 text-xs text-muted">
        {known
          ? `${instanceLine(known)}. Fills what you leave empty; what you set wins. Estimates: a vCPU counts as a full core and the network figure is the peak, so this is an upper bound on capacity, not on speed.`
          : "Fills the cores, the network card and the memory limit you leave empty, from a table of cloud instance types (estimates)."}
      </p>
    </div>
  );
}

/** A hard memory limit: how much the copy has, what one request holds, what it holds idle. */
export function MemoryFields({ name, config }: { name: string; config: NodeDoc }) {
  const patch = useDraft((s) => s.patch);
  const m = memoryOf(config);
  const inst = instanceOf(config);
  const fromInstance = inst !== null ? instanceByName(inst) : undefined;
  const slots =
    m.per_request !== null && (m.limit !== null || fromInstance)
      ? Math.floor(
          ((m.limit ?? (fromInstance ? fromInstance.gib * GIB : 0)) - (m.base ?? 0)) /
            m.per_request,
        )
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
        fromInstance ? `${fromInstance.gib} GiB (instance)` : "Not limited",
      )}
      {box(
        "per_request",
        "Memory per request",
        "What one request being served holds. Required for a limit to mean anything.",
        "Not set",
      )}
      {box("base", "Memory when idle", "What the process holds with no request. Default 0.", "0")}
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
