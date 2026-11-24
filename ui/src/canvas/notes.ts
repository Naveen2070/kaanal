// Captions pinned to the canvas (spec 5): a short note placed where it explains something. They are
// part of the draft view (`canvas_notes`) and are saved with the project in kaanal.layout.json,
// beside the positions (RULES 36(f)): an edit is shown at once and sent to the server like a move
// is, never recorded in undo. This file is the rules and the edits; the list itself is the draft's.

import type { DraftView, Note } from "../data/api/types";
import { OLD_SERVER_NOTES, useDraft } from "../data/store/draft";
import { notify } from "../shell/toast/toasts";

export type { Note };

/** Notes a project may hold, and the longest text of one (the server enforces the same). */
export const MAX_NOTES = 50;
export const MAX_NOTE_CHARS = 500;
/** A note's node id starts with this; a component's id is URI-encoded and never holds a raw colon. */
export const NOTE_PREFIX = "note:";
export const isNoteId = (id: string): boolean => id.startsWith(NOTE_PREFIX);

const NONE: Note[] = [];

/** The notes of the draft in force (every edit already made shows at once). */
export const useNotes = (): Note[] => useDraft((s) => s.view?.canvas_notes ?? NONE);

const current = (): Note[] => useDraft.getState().view?.canvas_notes ?? NONE;
const keep = (notes: Note[]) => void useDraft.getState().setNotes(notes);

/** A new empty note at `at`; returns its id, or null at the limit. */
export function addNoteAt(at: [number, number]): string | null {
  // an older server has no notes list and would drop the note (the store says so)
  if (useDraft.getState().view?.canvas_notes === undefined) {
    notify.warn(OLD_SERVER_NOTES, "notes-old-server");
    return null;
  }
  const notes = current();
  if (notes.length >= MAX_NOTES) return null;
  // the next number nobody holds, so an id is never reused while its note is on the page
  let n = notes.length + 1;
  while (notes.some((x) => x.id === `${NOTE_PREFIX}${n}`)) n++;
  const id = `${NOTE_PREFIX}${n}`;
  keep([...notes, { id, text: "", at }]);
  return id;
}

export const setNoteText = (id: string, text: string) =>
  keep(current().map((n) => (n.id === id ? { ...n, text: text.slice(0, MAX_NOTE_CHARS) } : n)));

export const moveNote = (id: string, at: [number, number]) =>
  keep(current().map((n) => (n.id === id ? { ...n, at } : n)));

export const removeNote = (id: string) => keep(current().filter((n) => n.id !== id));

const importKey = (project: string) => `kaanal-notes-${project}`;
const importedFlag = (project: string) => `kaanal-notes-imported-${project}`;
/** Projects whose browser notes were adopted into the draft of this server run (`boot`), not yet saved. */
const adopted = new Map<string, string>();
/** The server's coordinate limit (`MAX_COORD`) and id length (`MAX_ID_BYTES`). */
const MAX_COORD = 1e6;
const MAX_ID_BYTES = 64;

/**
 * Notes used to live in this browser (`kaanal-notes-<project_id>`). When the file has none, adopt
 * that copy into the draft (Save writes it). A file that has notes wins. The browser copy is never
 * deleted. The flag `kaanal-notes-imported-<project_id>` is set only once the draft is saved (or
 * the file already has notes, or the browser had none), so an import that was never saved is tried
 * again, and notes the user deleted from the file are never brought back. Blocked storage imports
 * nothing; an older server (no `canvas_notes`) would drop them, so nothing is done there.
 */
export function importBrowserNotes(view: DraftView | null): void {
  if (!view || view.canvas_notes === undefined) return;
  const project = view.project_id;
  try {
    if (localStorage.getItem(importedFlag(project)) !== null) return;
    const done = () => {
      adopted.delete(project);
      localStorage.setItem(importedFlag(project), "1");
    };
    if (adopted.get(project) === view.boot) {
      if (!view.layout_dirty) done(); // the adopted notes were saved (or deleted and saved)
      return;
    }
    if (view.canvas_notes.length > 0) {
      if (!view.layout_dirty) done(); // the file has notes: it wins
      return;
    }
    const mine = parseNotes(localStorage.getItem(importKey(project)));
    if (mine.length === 0) return done();
    adopted.set(project, view.boot);
    void useDraft
      .getState()
      .setNotes(mine)
      .then((ok) => {
        if (!ok) adopted.delete(project);
      });
  } catch {
    // blocked: nothing to import
  }
}

/** Whatever storage holds, kept only if it has the shape of notes (never trusted as it came). */
export function parseNotes(raw: string | null): Note[] {
  if (raw === null) return [];
  try {
    const v: unknown = JSON.parse(raw);
    if (!Array.isArray(v)) return [];
    const ids = new Set<string>();
    return v
      .flatMap((n): Note[] => {
        const o = n as Partial<Note> | null;
        const at = o?.at;
        // the same checks as the server's `Notes::check`, so a list it would refuse never goes out
        return o &&
          typeof o.id === "string" &&
          isNoteId(o.id) &&
          new TextEncoder().encode(o.id).length <= MAX_ID_BYTES &&
          !ids.has(o.id) &&
          ids.add(o.id) &&
          typeof o.text === "string" &&
          Array.isArray(at) &&
          at.length === 2 &&
          at.every((c) => typeof c === "number" && Number.isFinite(c) && Math.abs(c) <= MAX_COORD)
          ? [{ id: o.id, text: o.text.slice(0, MAX_NOTE_CHARS), at: [at[0], at[1]] }]
          : [];
      })
      .slice(0, MAX_NOTES);
  } catch {
    return [];
  }
}
