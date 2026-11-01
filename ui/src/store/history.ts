// Undo and redo for draft edits (spec 9). An edit is recorded as the patch that undoes it and the
// patch that does it again. The patches go through the draft store like any edit, so the server
// stays the owner of the file. The stack is cleared when the draft changes from outside (a reload, a
// file edit, a server restart), because the recorded patches no longer describe it. Moving a node
// is a layout change, not a patch, and is not recorded.

import { create } from "zustand";
import type { Doc, DraftView, Op } from "../api/types";
import { applyOps } from "./optimistic";

export const HISTORY_LIMIT = 100;

interface Entry {
  inverse: Op[];
  forward: Op[];
}

interface HistoryState {
  past: Entry[];
  future: Entry[];
  /** Said when the last edit could not be recorded; cleared by the next one that can. */
  note: string | null;
}

export const useHistory = create<HistoryState>(() => ({ past: [], future: [], note: null }));

export const history = {
  /** Records an edit that has been applied. A new edit ends the redo line. */
  push(inverse: Op[], forward: Op[]): void {
    useHistory.setState((s) => ({
      past: [...s.past, { inverse, forward }].slice(-HISTORY_LIMIT),
      future: [],
      note: null,
    }));
  },
  /** The patch that undoes the last edit (it moves to the redo line), or null with nothing to undo. */
  undo(): Op[] | null {
    const { past, future } = useHistory.getState();
    const e = past[past.length - 1];
    if (!e) return null;
    useHistory.setState({ past: past.slice(0, -1), future: [...future, e] });
    return e.inverse;
  },
  redo(): Op[] | null {
    const { past, future } = useHistory.getState();
    const e = future[future.length - 1];
    if (!e) return null;
    useHistory.setState({ future: future.slice(0, -1), past: [...past, e] });
    return e.forward;
  },
  clear(): void {
    useHistory.setState({ past: [], future: [], note: null });
  },
  /** An edit that cannot be undone: nothing is recorded, and the bar says so. */
  skip(): void {
    useHistory.setState({ note: "That edit can't be undone, so it isn't in the undo history." });
  },
};

type Path = (string | number)[];

function at(doc: unknown, path: Path): unknown {
  let x = doc;
  for (const step of path) {
    if (x === null || typeof x !== "object") return undefined;
    x = (x as Record<string | number, unknown>)[step];
  }
  return x;
}

/**
 * The patch that undoes `ops` on `view`, or null when some op cannot be undone. Worked out step by
 * step against the draft as it will be after each op, then reversed.
 */
export function invertOps(view: DraftView, ops: Op[]): Op[] | null {
  if (!view.doc) return null;
  const out: Op[] = [];
  let cur: DraftView = view;
  for (const o of ops) {
    const doc = cur.doc as Doc;
    switch (o.op) {
      case "set":
      case "insert": {
        const old = at(doc, o.path);
        const parent = at(doc, o.path.slice(0, -1));
        if (parent === null || typeof parent !== "object") return null;
        out.push(
          old === undefined
            ? { op: "remove", path: o.path }
            : { op: "set", path: o.path, value: structuredClone(old) },
        );
        break;
      }
      case "append": {
        const list = at(doc, o.path);
        if (!Array.isArray(list)) return null;
        out.push({ op: "remove", path: [...o.path, list.length] });
        break;
      }
      case "remove": {
        const old = at(doc, o.path);
        if (old === undefined) return null;
        const parentPath = o.path.slice(0, -1);
        const parent = at(doc, parentPath);
        // a list item goes back with its whole list (its index is not a stable address); a key goes back as itself
        out.push(
          Array.isArray(parent)
            ? { op: "set", path: parentPath, value: structuredClone(parent) }
            : { op: "insert", path: o.path, value: structuredClone(old) },
        );
        break;
      }
      case "add_node":
        if (doc.nodes && o.name in doc.nodes) return null;
        out.push({ op: "remove", path: ["nodes", o.name] });
        break;
      case "rename_node":
        out.push({ op: "rename_node", from: o.to, to: o.from });
        break;
    }
    cur = applyOps(cur, [o]);
  }
  return out.reverse();
}
