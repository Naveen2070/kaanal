// The "Quick setups" group of a Controls panel, for a wire and for a component: small outline buttons in
// one wrapping row (basic ones first, then advanced), the ones the thing already is shown as pressed,
// and what the single pressed one wrote under them. One click is one undo step. The group is open on
// something with nothing changed (a new user's first look) and folded once it has settings of its own;
// picking a setup keeps it open, so the click does not make the panel jump.

import { Group } from "./Group";
import { useGroups } from "./store";

export interface Chip {
  id: string;
  label: string;
  level: "basic" | "advanced";
  /** The tooltip, and the line under the chips when this is the only one pressed. */
  note: string;
  pressed: boolean;
}

const LEVELS = [
  ["basic", "Basic"],
  ["advanced", "Advanced"],
] as const;

export function SetupChips({
  groupId,
  chips,
  changed,
  onPick,
}: {
  /** The id the open state is kept under (`wire.quick` or `node.quick`). */
  groupId: string;
  chips: readonly Chip[];
  /** The thing already has settings of its own: the group starts folded. */
  changed: boolean;
  onPick: (id: string) => void;
}) {
  const pressed = chips.filter((c) => c.pressed);
  const only = pressed.length === 1 ? pressed[0] : undefined;
  return (
    <Group
      id={groupId}
      title="Quick setups"
      summary={pressed.length ? pressed.map((c) => c.label).join(" · ") : "none picked"}
      defaultOpen={!changed}
    >
      <div className="flex flex-wrap gap-1">
        {LEVELS.map(([level, name]) => (
          <fieldset key={level} className="m-0 contents border-0 p-0">
            <legend className="sr-only">{name} setups</legend>
            {chips
              .filter((c) => c.level === level)
              .map((c) => (
                <button
                  key={c.id}
                  type="button"
                  aria-pressed={c.pressed}
                  title={`${name}: ${c.note}`}
                  onClick={() => {
                    useGroups.getState().set(groupId, true);
                    onPick(c.id);
                  }}
                  className={`min-h-6 rounded-md border border-control px-1.5 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent ${
                    c.pressed ? "bg-active font-medium" : ""
                  }`}
                >
                  {c.label}
                </button>
              ))}
          </fieldset>
        ))}
      </div>
      {only ? <p className="m-0 mt-1 mb-1 text-xs text-muted">{only.note}</p> : null}
    </Group>
  );
}
