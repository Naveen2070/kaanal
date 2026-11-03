// The draft as the UI holds it. Edits show at once (optimistic) and go to the server one at a time;
// the view is always the last answer the server gave plus the edits still waiting, so a refused
// edit drops out without taking the later ones with it.

import { create } from "zustand";
import { ApiError, api } from "../api/client";
import type { DraftView, Layout, Op } from "../api/types";
import { connectOps, lastEdgeIndex, uniqueName } from "../../canvas/edges";
import { setKindRules } from "../../canvas/rules";
import { notify } from "../../shell/toast/toasts";
import { history, invertOps, useHistory } from "./history";
import { applyOps } from "./optimistic";

/** A patch may carry the layout it goes with (a new node and its spot are one edit). */
type Edit =
  | { kind: "patch"; ops: Op[]; layout?: Layout }
  | { kind: "layout"; layout: Layout }
  | { kind: "text"; text: string };

interface DraftState {
  view: DraftView | null;
  /** The last message of a refused edit or a failed load; cleared by the next success. */
  error: string | null;
  /** True once a load saw a different server `boot` than the one before: the server restarted. */
  restarted: boolean;
  load: () => Promise<void>;
  /** Resolves when every edit queued so far has been answered (Apply and Save wait for it). */
  settled: () => Promise<unknown>;
  /** Applies `ops` locally, sends them, and rolls them back if the server refuses. */
  patch: (ops: Op[], positions?: Layout, opts?: { record?: boolean }) => Promise<boolean>;
  /** Moves nodes: `positions` replaces the entries of those names; the whole map is sent. */
  setLayout: (positions: Layout) => Promise<boolean>;
  /** Replaces the whole draft text (the demo), after every edit already queued. */
  replaceText: (text: string) => Promise<boolean>;
  /** Adds a node of `kind` under a name not used yet; returns the name, or null when refused. */
  addNode: (kind: string, at?: [number, number]) => Promise<string | null>;
  /**
   * Draws an edge, then asks the server whether the draft still builds. An edge the draft cannot
   * have is taken back out and the reason returned; a problem elsewhere in the draft is returned
   * as a warning and the edge stays.
   */
  connect: (from: string, to: string) => Promise<ConnectResult>;
  /** Steps back / forward through the recorded edits; false when there is nothing to step to or it was refused. */
  undo: () => Promise<boolean>;
  redo: () => Promise<boolean>;
}

/** A step through the history worked: the problems it may have fixed are cleared at once, and it is said. */
function stepDone(word: string) {
  notify.clear("run");
  notify.clear("draft");
  notify.info(word, "history-step");
}

export type ConnectResult = { ok: true; warning?: string } | { ok: false; message: string };

function replay(confirmed: DraftView, pending: Edit[]): DraftView {
  return pending.reduce((v, e) => {
    // a whole-text replace has no local preview: the view keeps the old text until the answer
    if (e.kind === "text") return v;
    const next = e.kind === "patch" ? applyOps(v, e.ops) : v;
    return e.layout ? { ...next, layout: e.layout } : next;
  }, confirmed);
}

export const useDraft = create<DraftState>((set, get) => {
  let confirmed: DraftView | null = null;
  let pending: Edit[] = [];
  let tail: Promise<unknown> = Promise.resolve();

  // the stack changed (an edit, an undo): keep it with the draft it describes
  useHistory.subscribe(() => confirmed && history.save(confirmed.boot, confirmed.rev));

  const show = () => {
    if (!confirmed) return;
    setKindRules(confirmed.kinds);
    set({ view: replay(confirmed, pending) });
  };

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
            : edit.kind === "text"
              ? { text: edit.text }
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
        history.save(answer.boot, answer.rev);
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
        // a draft that changed under us (reload, file edit, restart) is not the one the history describes
        if (confirmed !== null && (restarted || confirmed.rev !== v.rev)) history.clear();
        // the first load of a page: take back the history this tab kept, if the draft is as it left it
        else if (confirmed === null) history.restore(v.boot, v.rev);
        confirmed = v;
        pending = [];
        set((s) => ({ error: null, restarted: s.restarted || restarted }));
        show();
      } catch (e) {
        set({ error: e instanceof ApiError ? e.message : String(e) });
      }
    },
    settled: () => tail,
    replaceText: (text) => enqueue({ kind: "text", text }),
    async patch(ops, positions, opts) {
      const before = get().view;
      const inverse = before && opts?.record !== false ? invertOps(before, ops) : null;
      const ok = await enqueue({
        kind: "patch",
        ops,
        layout: positions && { ...get().view?.layout, ...positions },
      });
      if (ok && opts?.record !== false) {
        if (inverse) history.push(inverse, ops);
        else history.skip();
      }
      return ok;
    },
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
      const ops = connectOps(doc, from, to);
      const inverse = invertOps(get().view as DraftView, ops);
      if (!(await get().patch(ops, undefined, { record: false }))) {
        return { ok: false, message: get().error ?? "the edge was refused" };
      }
      // the edge stays in every `ok: true` exit below, so the edit is recorded; a refused one never is
      const keep = () => {
        if (inverse) history.push(inverse, ops);
        else history.skip();
      };
      let check;
      try {
        check = await api.validate();
      } catch (e) {
        keep();
        return { ok: true, warning: e instanceof ApiError ? e.message : String(e) };
      }
      if (check.ok) {
        keep();
        return { ok: true };
      }
      const message = check.diagnostics[0]?.message ?? "the draft does not build";
      // the edge is taken back only when the problem is about it: its own entry, or the edge list
      // as a whole (too many edges); a problem in another edge or elsewhere was already there
      const now = get().view?.doc;
      const i = now ? lastEdgeIndex(now, from, to) : -1;
      const path = check.diagnostics[0]?.path ?? [];
      const aboutThis = path[0] === "edges" && (path.length === 1 || path[1] === i);
      if (!aboutThis || i < 0) {
        keep();
        return { ok: true, warning: message };
      }
      await get().patch([{ op: "remove", path: ["edges", i] }], undefined, { record: false });
      return { ok: false, message };
    },
    async undo() {
      const ops = history.undo();
      if (!ops) return false;
      const ok = await get().patch(ops, undefined, { record: false });
      // a step the draft refuses means the history is stale: drop it rather than offer more
      if (!ok) history.clear();
      else stepDone("Undone");
      return ok;
    },
    async redo() {
      const ops = history.redo();
      if (!ops) return false;
      const ok = await get().patch(ops, undefined, { record: false });
      if (!ok) history.clear();
      else stepDone("Redone");
      return ok;
    },
    setLayout(positions) {
      const view = get().view;
      return enqueue({ kind: "layout", layout: { ...view?.layout, ...positions } });
    },
  };
});
