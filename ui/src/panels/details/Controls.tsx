// The Controls tab for one node: what it can handle first (the job done most), when things go
// wrong, how busy it is, what that works out to, a suggested fix, right-now figures, then wiring,
// the name and delete. Slots and copies are set in the dock (ControlDock.tsx), one place each. Every edit is a patch of the draft; the auto re-run follows.

import { useId } from "react";
import { focusName } from "../../canvas/focusName";
import type { EdgeDoc, NodeDoc } from "../../data/api/types";
import { ConnectMenu, targetsFor, wiringNote } from "../../canvas/ConnectMenu";
import { fixNode, whyNodeRed, whyWireRed } from "../../canvas/insights";
import { bandwidthKbOf, EDGE_LIMITS } from "./edgeFields";
import { useFeed } from "../../canvas/feed";
import { nodeFigures, percent, replicasOf, twoSignificant } from "../../canvas/metrics";
import { kindInfo } from "../../canvas/nodeKinds";
import { LazyBoundary } from "../../kit/LazyBoundary";
import { useDraft } from "../../data/store/draft";
import { Chaos } from "./Chaos";
import { BalanceField, NumberField, Section, StyleField } from "./Field";
import { callsOthers } from "../../canvas/rules";
import { TrafficRoutes } from "./TrafficRoutes.lazy";
import { RATE_UNITS, TIME_UNITS } from "./units";
import {
  balanceOf,
  cardOf,
  deleteOps,
  holdOf,
  setStyle,
  durationMs,
  setBalance,
  setCard,
  hasProc,
  LIMITS,
  clearQueueLimit,
  queueOf,
  setDuration,
  setQueue,
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
  const why = node ? whyNodeRed(node) : null;
  const copyFix = node ? fixNode(node) : null;
  // a full wire into this component starves it: it shows idle, so say why
  const fullIn = feed.links?.find((l) => l.to === name && l.saturated);
  const incoming = fullIn
    ? edges.find((e) => String(e.from) === fullIn.from && String(e.to) === name)
    : undefined;
  const behindFull =
    fullIn && !node?.saturated
      ? whyWireRed(fullIn, incoming ? bandwidthKbOf(incoming) : null)
      : null;
  const calls = edges.filter((e) => String(e.from) === name).map((e) => String(e.to));
  const doc = useDraft((s) => s.view?.doc);
  // nothing to connect, no wire out and nothing to say about wiring: the section is left out
  const hasWiring =
    targetsFor(name, doc?.nodes ?? {}, edges).length > 0 ||
    calls.length > 0 ||
    wiringNote(name, doc?.nodes ?? {}, edges) !== null;

  return (
    <div className="p-3 text-sm">
      <Section title="Capacity">
        {replicas > 1 ? (
          <BalanceField
            hint="How calls into this component pick a copy. A wire into it can say its own."
            value={balanceOf(config.balance)}
            defaultLabel="Round robin (default)"
            onChange={(c) =>
              void patch(setBalance(["nodes", name, "balance"], "balance" in config, c))
            }
          />
        ) : null}
        {callsOthers(config.type) ? (
          <StyleField
            value={holdOf(config)}
            onChange={(c) => void patch(setStyle(name, config, c))}
          />
        ) : null}
        {!hasProc(config.type) ? (
          <p className="mb-2 text-xs text-muted">
            This kind prices its work per operation (reads, writes, hits), not with one service
            time. Edit those in kaanal.yaml for now.
          </p>
        ) : serviceMs !== null || !("proc" in config) ? (
          <NumberField
            label="Service time"
            units={TIME_UNITS}
            help="time"
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
          <p className="mb-2 text-xs text-muted">
            Service time is a spread of values (for example p50 and p99). Edit it in kaanal.yaml for
            now.
          </p>
        )}
        {queue !== null ? (
          <NumberField
            label="Queue limit"
            help="queue"
            hint="Requests that may wait for a slot. Past it they are turned away. Empty means no limit; 0 means nobody waits."
            value={queue === "unbounded" ? null : queue}
            placeholder="No limit"
            min={LIMITS.queue.min}
            max={LIMITS.queue.max}
            integer
            onCommit={(n) => patch(setQueue(name, config, n))}
          />
        ) : null}
        {typeof queue === "number" && clearQueueLimit(name, config).length > 0 ? (
          <button
            type="button"
            onClick={() => void patch(clearQueueLimit(name, config))}
            className="mb-2 min-h-6 rounded-md border border-control px-2 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
          >
            Remove the queue limit
          </button>
        ) : null}
      </Section>

      <Section title="When things go wrong">
        <NumberField
          label="Timeout"
          units={TIME_UNITS}
          hint="A caller gives up on this component after this long, wait and work included."
          value={timeoutMs}
          min={LIMITS.timeoutMs.min}
          max={LIMITS.timeoutMs.max}
          step="any"
          onCommit={(n) => patch([setDuration(name, config, "timeout", n)])}
        />
        <Chaos name={name} replicas={replicas} links={calls} onApplied={onChaos} />
      </Section>

      <Section title="How busy it is">
        <p>
          {percent(f.busy)} busy
          {f.peak !== null ? `, peaking at ${percent(f.peak)}` : ""}
        </p>
        <p className="text-xs text-muted">
          {f.hasRun
            ? "Measured over the run: the share of its slots in use."
            : "Nothing to show until a run has finished."}
        </p>
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
          <p className="text-xs text-muted">
            {node && spare !== undefined
              ? spare >= 0
                ? `${percent(spare)} spare at ${twoSignificant(node.visits_per_s)} visits/s.`
                : `Overloaded: ${twoSignificant(node.visits_per_s)} visits/s arrive, more than it can take.`
              : "Spare capacity shows after a run."}
          </p>
        </Section>
      ) : null}

      {config.type !== "cache" ? (
        <Section title="Network card">
          <NumberField
            label="Sends up to"
            units={RATE_UNITS}
            hint="What this component can send each second, over all its wires: calls out and replies. Each copy has its own. Empty means no limit."
            value={cardOf(config).send}
            placeholder="No limit"
            min={EDGE_LIMITS.bandwidthKb.min}
            max={EDGE_LIMITS.bandwidthKb.max}
            step="any"
            onCommit={(n) => patch(setCard(name, config, "send", n))}
          />
          {cardOf(config).send !== null ? (
            <button
              type="button"
              onClick={() => void patch(setCard(name, config, "send", null))}
              className="mb-2 min-h-6 rounded-md border border-control px-2 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
            >
              Remove the send limit
            </button>
          ) : null}
          <NumberField
            label="Receives up to"
            units={RATE_UNITS}
            hint="What this component can take in each second, over all its wires. Empty means no limit."
            value={cardOf(config).receive}
            placeholder="No limit"
            min={EDGE_LIMITS.bandwidthKb.min}
            max={EDGE_LIMITS.bandwidthKb.max}
            step="any"
            onCommit={(n) => patch(setCard(name, config, "receive", n))}
          />
          {cardOf(config).receive !== null ? (
            <button
              type="button"
              onClick={() => void patch(setCard(name, config, "receive", null))}
              className="mb-2 min-h-6 rounded-md border border-control px-2 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
            >
              Remove the receive limit
            </button>
          ) : null}
        </Section>
      ) : null}

      {why || behindFull ? (
        <Section title={why ? "Why is it red?" : "Why does it look idle?"}>
          {why ? <p>{why}</p> : null}
          {why && copyFix ? <p className="mt-1 text-xs text-muted">{copyFix}</p> : null}
          {!why && behindFull ? <p>{behindFull}</p> : null}
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

      {config.type === "gateway" && doc?.endpoints && Object.keys(doc.endpoints).length > 0 ? (
        <LazyBoundary fallback={<p className="mb-3 text-xs text-muted">Loading the routes...</p>}>
          <TrafficRoutes gateway={name} doc={doc} edges={edges} />
        </LazyBoundary>
      ) : null}

      {hasWiring ? (
        <Section title="Connect to…">
          <ConnectMenu from={name} />
        </Section>
      ) : null}

      <Section title="This component">
        <label htmlFor={nameId} className="block text-sm font-medium">
          Name
        </label>
        <input
          id={nameId}
          key={name}
          ref={(el) => {
            // a node just added from the palette: name ready to type over, once
            if (el && focusName.pending === name) {
              focusName.pending = null;
              el.focus();
              el.select();
            }
          }}
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
          className="h-7 w-full rounded border border-control bg-sheet text-text px-1"
        />
        <p className="mt-1 text-xs text-muted">
          {info ? `${info.label}: ${info.description}.` : config.type}
        </p>
      </Section>

      <button
        type="button"
        className="rounded border border-negative px-2 py-1 text-negative"
        onClick={() => {
          void patch(deleteOps(name, edges)).then((ok) => ok && onDeleted());
        }}
      >
        Delete component
      </button>
    </div>
  );
}
