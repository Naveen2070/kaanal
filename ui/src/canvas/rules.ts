// Who may call whom (R1.24), so the Studio offers only wires the draft will accept. The table is the
// engine's own `callees` (crates/kaanal-core/src/topology.rs), which the server sends with every draft
// view as `kinds`; the copy below is only what is used until that arrives (and in tests, and with an
// older server). The draft still has the last word.

import type { KindRules } from "../data/api/types";

let served: KindRules[] | undefined;

/** Called by the draft store whenever a view arrives. */
export function setKindRules(kinds: KindRules[] | undefined): void {
  served = kinds;
}

/** What the server says of a kind, when it has said. */
export function kindRulesOf(kind: string): KindRules | undefined {
  return served?.find((k) => k.kind === kind);
}

const BACKENDS = ["service", "monolith", "database", "cache", "external-api", "auth"];

const MAY_CALL: Record<string, string[]> = {
  gateway: ["service", "monolith", "auth"],
  service: BACKENDS,
  monolith: BACKENDS,
  auth: ["service", "monolith", "database", "cache", "external-api"],
  database: [],
  cache: [],
  "external-api": [],
};

/** Whether a node of this kind can call others at all (a database, a cache and an external API cannot). */
export function callsOthers(kind: string): boolean {
  return (kindRulesOf(kind)?.callees ?? MAY_CALL[kind] ?? []).length > 0;
}

/** True when a node of kind `from` may call one of kind `to`. An unknown kind is allowed: the draft decides. */
export function mayCall(from: string | undefined, to: string | undefined): boolean {
  if (from === undefined || to === undefined) return true;
  const allowed = kindRulesOf(from)?.callees ?? MAY_CALL[from];
  return allowed === undefined || allowed.includes(to);
}
