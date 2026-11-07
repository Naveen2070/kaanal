// The node kinds the core accepts (`type:` in kaanal.yaml; see `NodeKind` in kaanal-core). The
// server owns what a new node of a kind starts with (`kind_defaults`); this is only how the kind
// looks in the palette and on the canvas.

import {
  Archive,
  Box,
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
import type { KindRules } from "../data/api/types";

export interface KindInfo {
  /** The `type` tag and the name `add_node` takes. */
  kind: string;
  label: string;
  icon: LucideIcon;
  description: string;
}

export const NODE_KINDS: KindInfo[] = [
  {
    kind: "gateway",
    label: "Gateway",
    icon: DoorOpen,
    description: "The front door: where requests enter",
  },
  {
    kind: "service",
    label: "Service",
    icon: Network,
    description: "An app that does work and calls others",
  },
  {
    kind: "monolith",
    label: "Monolith",
    icon: Boxes,
    description: "One big app that does it all",
  },
  {
    kind: "database",
    label: "Database",
    icon: Database,
    description: "Stores data; limited by its connection pool",
  },
  {
    kind: "cache",
    label: "Cache",
    icon: Zap,
    description: "Answers repeat reads fast",
  },
  {
    kind: "external-api",
    label: "External API",
    icon: Globe,
    description: "Someone else's service you call",
  },
  {
    kind: "auth",
    label: "Auth",
    icon: KeyRound,
    description: "Checks who is calling",
  },
  {
    kind: "load-balancer",
    label: "Load balancer",
    icon: Split,
    description: "Sends each call to one of several components",
  },
  {
    kind: "storage",
    label: "Storage",
    icon: Archive,
    description: "Object storage like S3: a first byte, then the transfer",
  },
];

/** The kinds to offer: the server's registry when it has sent one (a kind the page has no look for gets a plain box), else the static list. */
export function offeredKinds(served: KindRules[] | undefined): KindInfo[] {
  if (!served || served.length === 0) return NODE_KINDS;
  return served.map(
    ({ kind }) =>
      kindInfo(kind) ?? {
        kind,
        label: kind,
        icon: Box,
        description: `A ${kind} node`,
      },
  );
}

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
  storage: "storage",
};

/** The CSS colour of a kind: its own hue, or a neutral for a kind the Studio does not know. */
export function catColour(kind: string): string {
  const h = HUE[kind];
  return h ? `var(--cat-${h})` : "var(--muted)";
}
