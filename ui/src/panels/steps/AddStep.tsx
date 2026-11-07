// The "add a step" pickers: a component to call, a cache to look up, and a parallel block. Each is one
// select and one button, and shows nothing when there is nothing to pick.

import { useState } from "react";

const SELECT =
  "min-w-0 flex-1 rounded-md border border-control bg-sheet px-1.5 py-1 font-mono text-xs focus-visible:outline-2 focus-visible:outline-accent";
const BUTTON =
  "shrink-0 rounded-md border border-control px-2 py-1 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent";

export function AddPick({
  options,
  selectLabel,
  button,
  onAdd,
  className = "",
  first = false,
}: {
  options: string[];
  selectLabel: string;
  button: string;
  onAdd: (node: string) => void;
  className?: string;
  /** The route's own first picker: where focus goes when its last step is deleted. */
  first?: boolean;
}) {
  const [node, setNode] = useState("");
  const chosen = options.includes(node) ? node : options[0];
  if (chosen === undefined) return null;
  return (
    <div className={`flex items-center gap-1.5 ${className}`}>
      <select
        aria-label={selectLabel}
        data-steps-add={first ? true : undefined}
        value={chosen}
        onChange={(e) => setNode(e.target.value)}
        className={SELECT}
      >
        {options.map((n) => (
          <option key={n} value={n}>
            {n}
          </option>
        ))}
      </select>
      <button type="button" onClick={() => onAdd(chosen)} className={BUTTON}>
        {button}
      </button>
    </div>
  );
}
