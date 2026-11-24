// Captions pinned to the canvas (spec 5): a short note placed where it explains something. They are
// part of the draft view (`canvas_notes`) and are saved with the project in kaanal.layout.json,
// beside the positions (RULES 36(f)): an edit is shown at once and sent to the server like a move
// is, never recorded in undo. This file is the rules and the edits; the list itself is the draft's.

import type { Note } from "../data/api/types";
import { useDraft } from "../data/store/draft";

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

/** Whatever storage holds, kept only if it has the shape of notes (never trusted as it came). */
export function parseNotes(raw: string | null): Note[] {
  if (raw === null) return [];
  try {
    const v: unknown = JSON.parse(raw);
    if (!Array.isArray(v)) return [];
    return v
      .flatMap((n): Note[] => {
        const o = n as Partial<Note> | null;
        const at = o?.at;
        return o &&
          typeof o.id === "string" &&
          isNoteId(o.id) &&
          typeof o.text === "string" &&
          Array.isArray(at) &&
          at.length === 2 &&
          at.every((c) => typeof c === "number" && Number.isFinite(c))
          ? [{ id: o.id, text: o.text.slice(0, MAX_NOTE_CHARS), at: [at[0], at[1]] }]
          : [];
      })
      .slice(0, MAX_NOTES);
  } catch {
    return [];
  }
}
