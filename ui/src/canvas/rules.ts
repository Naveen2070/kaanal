// Who may call whom (R1.24): the same table as `callees` in crates/kaanal-core/src/topology.rs, so
// the Studio offers only wires the draft will accept. The draft still has the last word.

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

/** True when a node of kind `from` may call one of kind `to`. An unknown kind is allowed: the draft decides. */
export function mayCall(from: string | undefined, to: string | undefined): boolean {
  if (from === undefined || to === undefined) return true;
  const allowed = MAY_CALL[from];
  return allowed === undefined || allowed.includes(to);
}
