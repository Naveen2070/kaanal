// A draft the notes tests can edit: a stub server that keeps the list the way the real one does
// (`PUT /draft` with `canvas_notes` replaces it, and the answer is the whole view).

import { vi } from "vite-plus/test";
import type { DraftView, Note } from "../data/api/types";
import { useDraft } from "../data/store/draft";

export function draftView(canvas_notes?: Note[]): DraftView {
  return {
    text: "",
    dirty: false,
    generation: 1,
    lost: [],
    rev: 0,
    boot: "b1",
    doc: { nodes: { gw: { type: "gateway" } }, edges: [] },
    doc_error: null,
    project_id: "proj",
    project_name: null,
    kind_defaults: {},
    layout: { gw: [0, 0] },
    layout_note: null,
    layout_dirty: false,
    saved: true,
    ...(canvas_notes && { canvas_notes }),
  };
}

export interface Request {
  method: string;
  body?: { canvas_notes?: Note[] };
}

/** Starts the stub and loads the draft; `refuse` makes every PUT a 400. */
export async function seedDraft(notes: Note[] = [], opts: { refuse?: boolean } = {}) {
  let held = notes;
  const sent: Request[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (_url: string, init: RequestInit = {}) => {
      const body = typeof init.body === "string" ? JSON.parse(init.body) : undefined;
      sent.push({ method: init.method ?? "GET", body });
      if (init.method === "PUT") {
        if (opts.refuse) return new Response(JSON.stringify({ error: "refused" }), { status: 400 });
        held = body.canvas_notes ?? held;
      }
      return new Response(JSON.stringify(draftView(held)), { status: 200 });
    }),
  );
  useDraft.setState({ view: null, error: null, restarted: false });
  await useDraft.getState().load();
  sent.length = 0;
  return sent;
}

export const notesOf = (): Note[] => useDraft.getState().view?.canvas_notes ?? [];
