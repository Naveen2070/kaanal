// The quick setups of a wire: three basic and three advanced (`presets.ts`), shown by `SetupChips`. The
// one the wire matches exactly shows as pressed, with what it wrote under it. One click is one undo
// step; the group is open on a wire with nothing changed and folded once it has settings of its own.

import { useDraft } from "../../../data/store/draft";
import type { EdgeDoc } from "../../../data/api/types";
import { GROUP } from "./ids";
import { matchingPreset, PRESETS, presetOps } from "./presets";
import { SetupChips } from "./SetupChips";
import { GROUP_KEYS, type WireGroup, wireGroupChanged } from "./summaries";

const anyChanged = (e: EdgeDoc): boolean =>
  (Object.keys(GROUP_KEYS) as WireGroup[]).some((g) => wireGroupChanged(g, e));

export function QuickSetups({ index, e }: { index: number; e: EdgeDoc }) {
  const patch = useDraft((s) => s.patch);
  const active = matchingPreset(e);
  return (
    <SetupChips
      groupId={GROUP.quick}
      chips={PRESETS.map((p) => ({ ...p, pressed: active === p.id }))}
      changed={anyChanged(e)}
      onPick={(id) => {
        const p = PRESETS.find((x) => x.id === id);
        const ops = p ? presetOps(p, index, e) : [];
        if (ops.length) void patch(ops);
      }}
    />
  );
}
