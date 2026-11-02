// Deleting wires from the canvas or from the rail: one edit (one undo), then a notice that offers
// the undo. A wire is also what lets an endpoint's chain call a service ("gateway has no edge to
// service" otherwise), so the calls that rode on a deleted wire are taken out of the chains in the
// same edit: the draft stays valid, and the notice says which endpoints stopped forwarding.

import type { Doc, Op } from "../data/api/types";
import { useDraft } from "../data/store/draft";
import { gatewayOf } from "../panels/traffic/model";
import { notify } from "../shell/toast/toasts";

type Obj = Record<string, unknown>;
const isObj = (x: unknown): x is Obj => x !== null && typeof x === "object" && !Array.isArray(x);

/**
 * `steps` without the calls from `from` to `to`. A top-level step is called by the gateway; a step
 * inside a call's own chain is called by that call's service; `parallel:` keeps its caller.
 */
export function prune(
  steps: unknown[],
  caller: string | null,
  from: string,
  to: string,
): unknown[] {
  const out: unknown[] = [];
  for (const st of steps) {
    if (!isObj(st)) {
      out.push(st);
    } else if (typeof st.call === "string") {
      if (caller === from && st.call === to) continue;
      out.push(Array.isArray(st.chain) ? { ...st, chain: prune(st.chain, st.call, from, to) } : st);
    } else if (Array.isArray(st.parallel)) {
      const inner = prune(st.parallel, caller, from, to);
      if (inner.length > 0) out.push({ ...st, parallel: inner });
    } else {
      out.push(st);
    }
  }
  return out;
}

/** The patch that takes the calls over the wires at `places` out of the endpoints, and which endpoints it touched. */
export function cascade(doc: Doc, places: number[]): { ops: Op[]; touched: string[] } {
  const gone = places
    .map((i) => doc.edges?.[i])
    .filter((e): e is NonNullable<typeof e> => e !== undefined)
    .map((e) => [String(e.from), String(e.to)] as const);
  const endpoints = doc.endpoints;
  const gateway = gatewayOf(doc);
  if (!isObj(endpoints) || gone.length === 0) return { ops: [], touched: [] };
  const ops: Op[] = [];
  const touched: string[] = [];
  for (const [key, e] of Object.entries(endpoints)) {
    if (!isObj(e) || !Array.isArray(e.chain)) continue;
    let chain: unknown[] = e.chain;
    for (const [from, to] of gone) chain = prune(chain, gateway, from, to);
    if (JSON.stringify(chain) === JSON.stringify(e.chain)) continue;
    touched.push(key);
    ops.push(
      chain.length === 0
        ? { op: "remove", path: ["endpoints", key, "chain"] }
        : { op: "set", path: ["endpoints", key, "chain"], value: chain },
    );
  }
  return { ops, touched };
}

/** Removes the `edges` entries at these places, highest first so the places stay true. */
export async function removeWires(places: number[]): Promise<void> {
  const at = [...new Set(places)].sort((a, b) => b - a);
  if (at.length === 0) return;
  const doc = useDraft.getState().view?.doc;
  const { ops: calls, touched } = doc ? cascade(doc, at) : { ops: [], touched: [] };
  const ok = await useDraft
    .getState()
    .patch([...calls, ...at.map((i): Op => ({ op: "remove", path: ["edges", i] }))]);
  if (!ok) return;
  const what = at.length === 1 ? "Wire deleted" : `${at.length} wires deleted`;
  const also =
    touched.length === 0
      ? ""
      : `; ${touched.join(", ")} no longer ${touched.length === 1 ? "forwards" : "forward"} there`;
  notify.info(`${what}${also}.`, "wire", {
    label: "Undo",
    run: () => void useDraft.getState().undo(),
  });
}
