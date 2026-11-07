// The parts a component's Controls shares in shape with a wire's: a sticky header (what it is, what
// is changed, the jump box), a strip of last-run figures, and the quick setups. Same look as the wire
// panel's, from the same pieces; only the words and the figures differ.

import { useDraft } from "../../data/store/draft";
import type { NodeDoc } from "../../data/api/types";
import { nodeFigures, percent, twoSignificant } from "../../canvas/metrics";
import { catColour, kindInfo } from "../../canvas/nodeKinds";
import type { NodeSummary } from "../../data/api/types";
import { Chip } from "./EffectStrip";
import { NODE_GROUP } from "./groups/ids";
import { JumpBox } from "./groups/JumpBox";
import { NODE_TARGETS } from "./groups/jump";
import { nodeAnyChanged, nodeChangedLine, nodeSetups } from "./groups/nodeSetups";
import { SetupChips } from "./groups/SetupChips";

/** The component's name and kind, what differs from the defaults, and the jump box; it stays while the panel scrolls. */
export function NodeHeader({
  name,
  config,
  more = [],
}: {
  name: string;
  config: NodeDoc;
  /** Changed groups kept outside the component's own config (the gateway's Steps). */
  more?: readonly string[];
}) {
  const info = kindInfo(config.type);
  const Icon = info?.icon;
  return (
    <div className="sticky top-0 z-10 -mx-3 -mt-3 mb-3 flex items-start gap-2 border-b bg-sheet px-3 pt-3 pb-3">
      <div className="min-w-0 flex-1">
        <p className="flex items-center gap-1.5 font-medium">
          {Icon ? (
            <span
              className="flex size-5 shrink-0 items-center justify-center rounded-md"
              style={{
                background: `color-mix(in srgb, ${catColour(config.type)} 16%, transparent)`,
              }}
            >
              <Icon size={12} aria-hidden style={{ color: catColour(config.type) }} />
            </span>
          ) : null}
          <span className="truncate">{name}</span>
          <span className="shrink-0 text-xs font-normal text-muted">
            {info?.label ?? config.type}
          </span>
        </p>
        <p className="mt-1 text-xs text-muted">{nodeChangedLine(config, more)}</p>
      </div>
      <JumpBox targets={NODE_TARGETS} />
    </div>
  );
}

/** Three figures of the last run for this component; nothing when the run did not reach it. */
export function NodeStrip({ node, config }: { node: NodeSummary | undefined; config: NodeDoc }) {
  if (!node) return null;
  const f = nodeFigures(node, config);
  const away = node.limited + node.overflowed;
  return (
    <section aria-label="Last run at this component" className="mb-3">
      <div className="grid grid-cols-3 gap-1.5">
        <Chip label="Busy" value={f.busy === null ? "–" : percent(f.busy)} />
        <Chip label="Turned away" value={away.toLocaleString("en-US")} />
        <Chip label="Average wait" value={`${twoSignificant(node.mean_wait_ms)} ms`} />
      </div>
      <p className="m-0 mt-1 text-[11px] text-muted">
        From the last run: {twoSignificant(node.visits_per_s)} requests a second arrived here.
      </p>
    </section>
  );
}

/** One-click setups for this kind of component (Tomcat's, HikariCP's and PostgreSQL's published defaults). */
export function NodeSetups({ name, config }: { name: string; config: NodeDoc }) {
  const patch = useDraft((s) => s.patch);
  const setups = nodeSetups(name, config);
  return (
    <SetupChips
      groupId={NODE_GROUP.quick}
      chips={setups}
      changed={nodeAnyChanged(config)}
      onPick={(id) => {
        const ops = setups.find((s) => s.id === id)?.ops ?? [];
        if (ops.length) void patch(ops);
      }}
    />
  );
}
