// Captions pinned to the canvas (spec 5): a short note placed where it explains something. They are
// kept per project in this browser (the draft's `project_id`), not in kaanal.yaml or the layout
// file, so they are not saved with the project, shared, or part of a Save. Moving them into the
// layout sidecar is the way to share them (RULES 36). Storage can be blocked: then they last for
// the page and the canvas works the same.

import { create } from "zustand";

export interface Note {
  id: string;
  text: string;
  /** Top-left corner, in flow coordinates. */
  at: [number, number];
}

/** Notes a project may hold, and the longest text of one. */
export const MAX_NOTES = 50;
export const MAX_NOTE_CHARS = 500;
/** A note's node id starts with this; a component's id is URI-encoded and never holds a raw colon. */
export const NOTE_PREFIX = "note:";
export const isNoteId = (id: string): boolean => id.startsWith(NOTE_PREFIX);

const key = (project: string) => `kaanal-notes-${project}`;

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

interface NotesState {
  project: string | null;
  notes: Note[];
  /** Reads the notes of a project (once per project). */
  load: (project: string) => void;
  /** A new empty note at `at`; returns its id, or null at the limit. */
  add: (at: [number, number]) => string | null;
  setText: (id: string, text: string) => void;
  move: (id: string, at: [number, number]) => void;
  remove: (id: string) => void;
}

export const useNotes = create<NotesState>((set, get) => {
  const keep = (notes: Note[]) => {
    set({ notes });
    const project = get().project;
    if (project === null) return;
    try {
      localStorage.setItem(key(project), JSON.stringify(notes));
    } catch {
      // blocked or full: the notes stay on this page
    }
  };
  return {
    project: null,
    notes: [],
    load(project) {
      if (get().project === project) return;
      let raw: string | null = null;
      try {
        raw = localStorage.getItem(key(project));
      } catch {
        // blocked: start empty
      }
      set({ project, notes: parseNotes(raw) });
    },
    add(at) {
      const { notes } = get();
      if (notes.length >= MAX_NOTES) return null;
      // the next number nobody holds, so an id is never reused while its note is on the page
      let n = notes.length + 1;
      while (notes.some((x) => x.id === `${NOTE_PREFIX}${n}`)) n++;
      const id = `${NOTE_PREFIX}${n}`;
      keep([...notes, { id, text: "", at }]);
      return id;
    },
    setText: (id, text) =>
      keep(
        get().notes.map((n) => (n.id === id ? { ...n, text: text.slice(0, MAX_NOTE_CHARS) } : n)),
      ),
    move: (id, at) => keep(get().notes.map((n) => (n.id === id ? { ...n, at } : n))),
    remove: (id) => keep(get().notes.filter((n) => n.id !== id)),
  };
});
