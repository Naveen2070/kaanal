// What a patch will do to the draft's JSON, worked out locally so the canvas does not wait for the
// server. The server's answer replaces it; a patch it refuses is rolled back (draft.ts).

import type { Doc, DraftView, Layout, Op } from "../api/types";

type Json = unknown;

function child(parent: Json, step: string | number): Json {
  if (parent === null || typeof parent !== "object") return undefined;
  return (parent as Record<string | number, Json>)[step];
}

function setAt(doc: Doc, path: (string | number)[], value: Json): void {
  let at: Json = doc;
  for (const step of path.slice(0, -1)) at = child(at, step);
  const last = path[path.length - 1];
  if (at === null || typeof at !== "object" || last === undefined) return;
  (at as Record<string | number, Json>)[last] = structuredClone(value);
}

function removeAt(doc: Doc, path: (string | number)[]): void {
  let at: Json = doc;
  for (const step of path.slice(0, -1)) at = child(at, step);
  const last = path[path.length - 1];
  if (last === undefined) return;
  if (Array.isArray(at) && typeof last === "number") at.splice(last, 1);
  else if (at !== null && typeof at === "object") delete (at as Record<string, Json>)[String(last)];
}

function renameNode(doc: Doc, layout: Layout, from: string, to: string): void {
  if (doc.nodes && from in doc.nodes) {
    // keep the order of the keys: rebuild the map
    doc.nodes = Object.fromEntries(
      Object.entries(doc.nodes).map(([k, v]) => [k === from ? to : k, v]),
    );
  }
  for (const e of doc.edges ?? []) {
    if (String(e.from) === from) e.from = to;
    if (String(e.to) === from) e.to = to;
  }
  const p = layout[from];
  if (p) {
    delete layout[from];
    layout[to] = p;
  }
}

/** `view` with `ops` applied to its `doc` (a copy). A view with no doc (half-typed text) is unchanged. */
export function applyOps(view: DraftView, ops: Op[]): DraftView {
  if (!view.doc) return view;
  const doc = structuredClone(view.doc);
  const layout = { ...view.layout };
  for (const o of ops) {
    switch (o.op) {
      case "set":
      case "insert":
        setAt(doc, o.path, o.value);
        break;
      case "append": {
        let at: Json = doc;
        for (const step of o.path) at = child(at, step);
        if (Array.isArray(at)) at.push(structuredClone(o.value));
        break;
      }
      case "remove":
        removeAt(doc, o.path);
        break;
      case "add_node": {
        const start = view.kind_defaults[o.kind];
        if (start) (doc.nodes ??= {})[o.name] = structuredClone(start);
        break;
      }
      case "rename_node":
        renameNode(doc, layout, o.from, o.to);
        break;
    }
  }
  return { ...view, doc, layout };
}
