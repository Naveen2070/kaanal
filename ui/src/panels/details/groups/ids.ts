// The ids the wire's groups keep their open state under (`store.ts`). One list, so a test can open
// them all and the jump box can open the one that holds a setting.

export const GROUP = {
  quick: "wire.quick",
  connection: "wire.connection",
  compress: "wire.compress",
  retry: "wire.retry",
  breaker: "wire.breaker",
  usage: "wire.usage",
} as const;

/** The groups of a component's panel. */
export const NODE_GROUP = {
  quick: "node.quick",
  network: "node.network",
  limit: "node.limit",
  balance: "node.balance",
  style: "node.style",
  steps: "node.steps",
} as const;

/** The groups of the panel with nothing selected. */
export const DOC_GROUP = { headers: "doc.headers" } as const;

/** The folded groups at the foot of Results. */
export const RESULTS_GROUP = {
  goals: "results.goals",
  share: "results.share",
  notes: "results.notes",
} as const;

export const ALL_WIRE_GROUPS: string[] = Object.values(GROUP);

export const ALL_NODE_GROUPS: string[] = Object.values(NODE_GROUP);

export const ALL_RESULTS_GROUPS: string[] = Object.values(RESULTS_GROUP);
