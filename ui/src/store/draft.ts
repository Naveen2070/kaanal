// The draft as the UI holds it. Edits show at once (optimistic) and go to the server one at a time;
// the view is always the last answer the server gave plus the edits still waiting, so a refused
// edit drops out without taking the later ones with it.

import { create } from "zustand";
import { ApiError, api } from "../api/client";
import type { DraftView, Layout, Op } from "../api/types";
import { connectOps, lastEdgeIndex, uniqueName } from "../canvas/edges";
import { applyOps } from "./optimistic";

/** A patch may carry the layout it goes with (a new node and its spot are one edit). */
type Edit = { kind: "patch"; ops: Op[]; layout?: Layout } | { kind: "layout"; layout: Layout };

interface DraftState {
  view: DraftView | null;
  /** The last message of a refused edit or a failed load; cleared by the next success. */
  error: string | null;
  /** True once a load saw a different server `boot` than the one before: the server restarted. */
  restarted: boolean;
  load: () => Promise<void>;
  /** Applies `ops` locally, sends them, and rolls them back if the server refuses. */
  patch: (ops: Op[], positions?: Layout) => Promise<boolean>;
  /** Moves nodes: `positions` replaces the entries of those names; the whole map is sent. */
  setLayout: (positions: Layout) => Promise<boolean>;
  /** Adds a node of `kind` under a name not used yet; returns the name, or null when refused. */
  addNode: (kind: string, at?: [number, number]) => Promise<string | null>;
  /**
   * Draws an edge, then asks the server whether the draft still builds. An edge the draft cannot
   * have is taken back out and the reason returned; a problem elsewhere in the draft is returned
   * as a warning and the edge stays.
   */
  connect: (from: string, to: string) => Promise<ConnectResult>;
}

export type ConnectResult = { ok: true; warning?: string } | { ok: false; message: string };

function replay(confirmed: DraftView, pending: Edit[]): DraftView {
  return pending.reduce((v, e) => {
    const next = e.kind === "patch" ? applyOps(v, e.ops) : v;
    return e.layout ? { ...next, layout: e.layout } : next;
  }, confirmed);
}

export const useDraft = create<DraftState>((set, get) => {
  let confirmed: DraftView | null = null;
  let pending: Edit[] = [];
  let tail: Promise<unknown> = Promise.resolve();

  const show = () => confirmed && set({ view: replay(confirmed, pending) });

  /** Queues `edit`: shown now, sent after the edits before it. */
  function enqueue(edit: Edit): Promise<boolean> {
    if (!confirmed) return Promise.resolve(false);
    pending = [...pending, edit];
    show();
    const run = async (): Promise<boolean> => {
      try {
        const answer = await api.putDraft(
          edit.kind === "patch"
            ? { patch: edit.ops, ...(edit.layout && { layout: edit.layout }) }
            : { layout: edit.layout },
        );
        pending = pending.filter((e) => e !== edit);
        // the server stores any text, so an edit can leave a draft that is not a config at all and
        // the canvas would go blank with no way back: put the last good text back and say why
        if (edit.kind === "patch" && confirmed?.doc && answer.doc === null) {
          const why = answer.doc_error?.message ?? "the draft no longer parses";
          confirmed = await api.putDraft({ text: confirmed.text });
          set({ error: `That change makes the draft invalid, so it was undone: ${why}` });
          show();
          return false;
        }
        confirmed = answer;
        set({ error: null });
        show();
        return true;
      } catch (e) {
        pending = pending.filter((p) => p !== edit);
        set({ error: e instanceof ApiError ? e.message : String(e) });
        show();
        return false;
      }
    };
    const done = tail.then(run);
    tail = done;
    return done;
  }

  return {
    view: null,
    error: null,
    restarted: false,
    async load() {
      try {
        const v = await api.getDraft();
        const restarted = confirmed !== null && confirmed.boot !== v.boot;
        confirmed = v;
        pending = [];
        set((s) => ({ error: null, restarted: s.restarted || restarted }));
        show();
      } catch (e) {
        set({ error: e instanceof ApiError ? e.message : String(e) });
      }
    },
    patch: (ops, positions) =>
      enqueue({
        kind: "patch",
        ops,
        layout: positions && { ...get().view?.layout, ...positions },
      }),
    async addNode(kind, at) {
      const doc = get().view?.doc;
      if (!doc) return null;
      const name = uniqueName(kind, Object.keys(doc.nodes ?? {}));
      // no spot given (a click, not a drop): to the right of everything drawn
      const placed = Object.values(get().view?.layout ?? {});
      const x = placed.length ? Math.max(...placed.map((p) => p[0])) + 280 : 0;
      const ok = await get().patch([{ op: "add_node", name, kind }], { [name]: at ?? [x, 0] });
      return ok ? name : null;
    },
    async connect(from, to) {
      const doc = get().view?.doc;
      if (!doc) return { ok: false, message: "the draft is not a valid config yet" };
      if (!(await get().patch(connectOps(doc, from, to)))) {
        return { ok: false, message: get().error ?? "the edge was refused" };
      }
      let check;
      try {
        check = await api.validate();
      } catch (e) {
        return { ok: true, warning: e instanceof ApiError ? e.message : String(e) };
      }
      if (check.ok) return { ok: true };
      const message = check.diagnostics[0]?.message ?? "the draft does not build";
      // the edge is taken back only when the problem is about it: its own entry, or the edge list
      // as a whole (too many edges); a problem in another edge or elsewhere was already there
      const now = get().view?.doc;
      const i = now ? lastEdgeIndex(now, from, to) : -1;
      const path = check.diagnostics[0]?.path ?? [];
      const aboutThis = path[0] === "edges" && (path.length === 1 || path[1] === i);
      if (!aboutThis || i < 0) return { ok: true, warning: message };
      await get().patch([{ op: "remove", path: ["edges", i] }]);
      return { ok: false, message };
    },
    setLayout(positions) {
      const view = get().view;
      return enqueue({ kind: "layout", layout: { ...view?.layout, ...positions } });
    },
  };
});
