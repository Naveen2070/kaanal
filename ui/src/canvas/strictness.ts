// What a run leaves out when the model is at its default strictness: a slot held while waiting on a
// call (`hold: call`) and network bandwidth. Both are real limits; without them a run is optimistic.

import type { Doc } from "../data/api/types";

const HOLD = "a slot held while waiting on a call (Server style)";
const BANDWIDTH = "network bandwidth";
const TAIL = "A real system may saturate earlier.";

const set = (b: unknown) => b !== undefined && b !== null;

/** The note for the draft, or null when both limits are on or nothing calls anything. */
export function strictnessNote(doc: Doc | null | undefined): string | null {
  if (!doc || (doc.edges?.length ?? 0) === 0) return null;
  const nodes = Object.values(doc.nodes ?? {});
  const hold = nodes.some((n) => n.hold === "call");
  const bandwidth = nodes.some((n) => set(n.bandwidth)) || doc.edges!.some((e) => set(e.bandwidth));
  if (hold && bandwidth) return null;
  const left = hold ? BANDWIDTH : bandwidth ? HOLD : `two limits: ${HOLD} and ${BANDWIDTH}`;
  return `This run leaves out ${left}. ${TAIL}`;
}
