// A node on the canvas. This is the plain card (name and kind); the busy bar, queue sparkline and
// quick controls are added in Task C1.

import { Handle, Position, type NodeProps } from "@xyflow/react";
import { kindInfo } from "./nodeKinds";

export interface CardData extends Record<string, unknown> {
  /** The node's real name (the React Flow id is its encoded form). */
  name: string;
  kind: string;
}

export function NodeCard({ data, selected }: NodeProps) {
  const { name, kind } = data as CardData;
  const info = kindInfo(kind);
  const Icon = info?.icon;
  return (
    <div
      className={`min-w-40 rounded-lg border-2 bg-white px-3 py-2 shadow-sm dark:bg-neutral-900 ${
        info?.accent ?? "border-neutral-400 text-neutral-600"
      } ${selected ? "ring-2 ring-blue-500" : ""}`}
    >
      <Handle type="target" position={Position.Left} />
      <div className="flex items-center gap-2">
        {Icon ? <Icon size={16} aria-hidden /> : null}
        <span className="truncate font-medium text-neutral-900 dark:text-neutral-100">{name}</span>
      </div>
      <div className="text-xs text-neutral-500">{info?.label ?? kind}</div>
      <Handle type="source" position={Position.Right} />
    </div>
  );
}
