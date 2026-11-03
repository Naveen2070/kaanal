// The node kinds the core accepts (`type:` in kaanal.yaml; see `NodeKind` in kaanal-core). The
// server owns what a new node of a kind starts with (`kind_defaults`); this is only how the kind
// looks in the palette and on the canvas.

import {
  Boxes,
  Database,
  DoorOpen,
  Globe,
  KeyRound,
  Network,
  Split,
  Zap,
  type LucideIcon,
} from "lucide-react";

export interface KindInfo {
  /** The `type` tag and the name `add_node` takes. */
  kind: string;
  label: string;
  icon: LucideIcon;
  /** Tailwind classes for the accent (border and icon). */
  accent: string;
  description: string;
}

export const NODE_KINDS: KindInfo[] = [
  {
    kind: "gateway",
    label: "Gateway",
    icon: DoorOpen,
    accent: "border-sky-500 text-sky-600",
    description: "The front door: where requests enter",
  },
  {
    kind: "service",
    label: "Service",
    icon: Network,
    accent: "border-emerald-500 text-emerald-600",
    description: "An app that does work and calls others",
  },
  {
    kind: "monolith",
    label: "Monolith",
    icon: Boxes,
    accent: "border-teal-500 text-teal-600",
    description: "One big app that does it all",
  },
  {
    kind: "database",
    label: "Database",
    icon: Database,
    accent: "border-violet-500 text-violet-600",
    description: "Stores data; limited by its connection pool",
  },
  {
    kind: "cache",
    label: "Cache",
    icon: Zap,
    accent: "border-amber-500 text-amber-600",
    description: "Answers repeat reads fast",
  },
  {
    kind: "external-api",
    label: "External API",
    icon: Globe,
    accent: "border-rose-500 text-rose-600",
    description: "Someone else's service you call",
  },
  {
    kind: "auth",
    label: "Auth",
    icon: KeyRound,
    accent: "border-orange-500 text-orange-600",
    description: "Checks who is calling",
  },
  {
    kind: "load-balancer",
    label: "Load balancer",
    icon: Split,
    accent: "border-yellow-500 text-yellow-600",
    description: "Sends each call to one of several components",
  },
];

export function kindInfo(kind: string | undefined): KindInfo | undefined {
  return NODE_KINDS.find((k) => k.kind === kind);
}

/** Kinds with a hue of their own (tokens.css `--cat-*`); `external-api` is `--cat-external`. */
const HUE: Record<string, string> = {
  gateway: "gateway",
  service: "service",
  monolith: "monolith",
  database: "database",
  cache: "cache",
  "external-api": "external",
  auth: "auth",
  "load-balancer": "balancer",
};

/** The CSS colour of a kind: its own hue, or a neutral for a kind the Studio does not know. */
export function catColour(kind: string): string {
  const h = HUE[kind];
  return h ? `var(--cat-${h})` : "var(--muted)";
}
