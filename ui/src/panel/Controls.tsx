// The Controls tab for one node, top to bottom as the spec lists it (7.1): name and what it is,
// how busy, what it can handle, when things go wrong, what that works out to, a suggested fix,
// right-now figures, delete. Every edit is a patch of the draft; the auto re-run follows.

import { useId } from "react";
import type { EdgeDoc, NodeDoc } from "../api/types";
import { useFeed } from "../canvas/feed";
import { nodeFigures, percent, replicasOf, twoSignificant } from "../canvas/metrics";
import { kindInfo } from "../canvas/nodeKinds";
import { useDraft } from "../store/draft";
import { Chaos } from "./Chaos";
import { NumberField, Section } from "./Field";
import {
  deleteOps,
  durationMs,
  hasProc,
  LIMITS,
  queueOf,
  setDuration,
  setQueue,
  setReplicas,
  setSlots,
  slotsOf,
  suggestedFix,
  worksOutTo,
} from "./fields";

export function Controls({
  name,
  config,
  edges,
  onDeleted,
  onRenamed,
  onChaos,
}: {
  name: string;
  config: NodeDoc;
  edges: EdgeDoc[];
  onDeleted: () => void;
  /** The node is called `to` now: the panel follows it. */
  onRenamed: (to: string) => void;
  /** Live chaos changed: the numbers on screen are out of date. */
  onChaos: () => void;
}) {
  const patch = useDraft((s) => s.patch);
  const feed = useFeed();
  const nameId = useId();
  const node = feed.node(name);
  const f = nodeFigures(node, config);
  const info = kindInfo(config.type);
  const slots = slotsOf(config);
  const queue = queueOf(config);
  const serviceMs = durationMs(config.proc);
  const replicas = replicasOf(config);
  const timeoutMs = durationMs(config.timeout);
  const out =
    slots.value !== null && serviceMs !== null && serviceMs > 0
      ? worksOutTo(slots.value, serviceMs, replicas, node?.visits_per_s ?? 0)
      : null;
  // after a run, the measured capacity (mean cost per visit, payload and own hops included) wins
  const capacity = node?.capacity_per_s ?? out?.capacityPerS ?? null;
  const spare =
    node && capacity !== null && capacity > 0 ? 1 - node.visits_per_s / capacity : out?.spare;
  const fix = suggestedFix(node);
  const calls = edges.filter((e) => String(e.from) === name).map((e) => String(e.to));
  const doc = useDraft((s) => s.view?.doc);

  return (
    <div className="p-3 text-sm">
      <Section title="This component">
        <label htmlFor={nameId} className="block text-sm font-medium">
          Name
        </label>
        <input
          id={nameId}
          key={name}
          defaultValue={name}
          onBlur={(e) => {
            const input = e.currentTarget;
            const to = input.value.trim();
            if (!to || to === name) return;
            void patch([{ op: "rename_node", from: name, to }]).then((ok) => {
              if (ok) onRenamed(to);
              else input.value = name;
            });
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") e.currentTarget.blur();
          }}
          className="w-full rounded border px-1"
        />
        <p className="mt-1 text-xs text-neutral-500">
          {info ? `${info.label}: ${info.description}.` : config.type}
        </p>
      </Section>

      {config.type === "gateway" && doc?.endpoints && Object.keys(doc.endpoints).length > 0 ? (
        <Section title="Traffic routes">
          <p className="mb-2 text-xs text-neutral-500">
            Where incoming endpoints forward requests.
          </p>
          {Object.entries(doc.endpoints).map(([epKey, epVal]) => {
            const epObj =
              epVal && typeof epVal === "object" ? (epVal as Record<string, unknown>) : {};
            const chain = Array.isArray(epObj.chain) ? epObj.chain : [];
            const firstCall =
              chain[0] && typeof chain[0] === "object"
                ? (chain[0] as { call?: unknown }).call
                : undefined;
            const currentTarget = typeof firstCall === "string" ? firstCall : "";
            const options = Array.from(
              new Set(["", ...calls, ...(currentTarget ? [currentTarget] : [])]),
            );
            const selectId = `route-${epKey}`;

            return (
              <div key={epKey} className="mb-2">
                <label
                  htmlFor={selectId}
                  className="block text-xs font-mono font-medium text-neutral-700"
                >
                  {epKey}
                </label>
                <select
                  id={selectId}
                  className="mt-1 w-full rounded border bg-white px-1 py-0.5 text-xs"
                  value={currentTarget}
                  onChange={(e) => {
                    const to = e.currentTarget.value;
                    if (to === currentTarget) return;
                    if (to === "") {
                      if ("chain" in epObj) {
                        void patch([{ op: "remove", path: ["endpoints", epKey, "chain"] }]);
                      }
                    } else {
                      const opKind = "chain" in epObj ? "set" : "insert";
                      void patch([
                        {
                          op: opKind,
                          path: ["endpoints", epKey, "chain"],
                          value: [{ call: to }],
                        },
                      ]);
                    }
                  }}
                >
                  <option value="">(Gateway answers directly)</option>
                  {options
                    .filter((target): target is string => Boolean(target))
                    .map((target) => (
                      <option key={target} value={target}>
                        Forward to {target}
                      </option>
                    ))}
                </select>
              </div>
            );
          })}
        </Section>
      ) : null}

      <Section title="How busy it is">
        <p>
          {percent(f.busy)} busy
          {f.peak !== null ? `, peaking at ${percent(f.peak)}` : ""}
        </p>
        <p className="text-xs text-neutral-500">
          {f.hasRun
            ? "Measured over the run: the share of its slots in use."
            : "Nothing to show until a run has finished."}
        </p>
      </Section>

      <Section title="How much it can handle">
        <NumberField
          label="Slots per instance"
          hint="How many requests it works on at once. More slots, more throughput."
          value={slots.value}
          min={LIMITS.slots.min}
          max={LIMITS.slots.max}
          integer
          onCommit={(n) => patch(setSlots(name, config, n))}
        />
        <NumberField
          label="Instances"
          hint="Copies of this component, each with its own slots and queue."
          value={replicas}
          min={LIMITS.instances.min}
          max={LIMITS.instances.max}
          integer
          onCommit={(n) => patch([setReplicas(name, config, n)])}
        />
        {!hasProc(config.type) ? (
          <p className="mb-2 text-xs text-neutral-500">
            This kind prices its work per operation (reads, writes, hits), not with one service
            time. Edit those in kaanal.yaml for now.
          </p>
        ) : serviceMs !== null || !("proc" in config) ? (
          <NumberField
            label="Service time (ms)"
            hint={
              serviceMs === null && slots.value !== null
                ? "Not set: this component never saturates, however much load you add. Set a time."
                : "How long one request takes once it has a slot."
            }
            value={serviceMs}
            min={LIMITS.serviceMs.min}
            max={LIMITS.serviceMs.max}
            step="any"
            onCommit={(n) => patch([setDuration(name, config, "proc", n)])}
          />
        ) : (
          <p className="mb-2 text-xs text-neutral-500">
            Service time is a spread of values (for example p50 and p99). Edit it in kaanal.yaml for
            now.
          </p>
        )}
        {queue !== null ? (
          <NumberField
            label="Queue limit"
            hint="Requests that may wait for a slot. Past it they are turned away."
            value={queue}
            min={LIMITS.queue.min}
            max={LIMITS.queue.max}
            integer
            onCommit={(n) => patch(setQueue(name, config, n))}
          />
        ) : null}
      </Section>

      <Section title="When things go wrong">
        <NumberField
          label="Timeout (ms)"
          hint="A caller gives up on this component after this long, wait and work included."
          value={timeoutMs}
          min={LIMITS.timeoutMs.min}
          max={LIMITS.timeoutMs.max}
          step="any"
          onCommit={(n) => patch([setDuration(name, config, "timeout", n)])}
        />
        <Chaos name={name} replicas={replicas} links={calls} onApplied={onChaos} />
      </Section>

      {out ? (
        <Section title="What that works out to">
          <p>
            {slots.value} at once x {replicas} instance{replicas === 1 ? "" : "s"} / {serviceMs} ms
            each = {twoSignificant(out.capacityPerS)} visits/s
            {node?.capacity_per_s != null && Math.abs(node.capacity_per_s - out.capacityPerS) > 0.5
              ? ` (the run measured ${twoSignificant(node.capacity_per_s)} visits/s: payload and own hops cost extra)`
              : ""}
          </p>
          <p className="text-xs text-neutral-500">
            {node && spare !== undefined
              ? spare >= 0
                ? `${percent(spare)} spare at ${twoSignificant(node.visits_per_s)} visits/s.`
                : `Overloaded: ${twoSignificant(node.visits_per_s)} visits/s arrive, more than it can take.`
              : "Spare capacity shows after a run."}
          </p>
        </Section>
      ) : null}

      {fix ? (
        <Section title="Suggested fix">
          <p>{fix}</p>
        </Section>
      ) : null}

      {node ? (
        <Section title="Right now (the run's window)">
          <dl className="grid grid-cols-2 gap-x-2 text-xs">
            <dt>Arriving</dt>
            <dd>{twoSignificant(node.visits_per_s)} / s</dd>
            <dt>Longest queue</dt>
            <dd>{node.max_queue}</dd>
            <dt>Average wait</dt>
            <dd>{twoSignificant(node.mean_wait_ms)} ms</dd>
            <dt>Turned away</dt>
            <dd>{node.limited + node.overflowed}</dd>
            <dt>Timed out</dt>
            <dd>{node.timed_out}</dd>
          </dl>
        </Section>
      ) : null}

      <button
        type="button"
        className="rounded border border-red-600 px-2 py-1 text-red-700"
        onClick={() => {
          void patch(deleteOps(name, edges)).then((ok) => ok && onDeleted());
        }}
      >
        Delete component
      </button>
    </div>
  );
}
