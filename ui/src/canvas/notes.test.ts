import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { isNoteId, MAX_NOTE_CHARS, MAX_NOTES, parseNotes, useNotes } from "./notes";

beforeEach(() => {
  localStorage.clear();
  useNotes.setState({ project: null, notes: [] });
});
afterEach(() => vi.unstubAllGlobals());

test("a_note_is_added_moved_edited_and_removed_and_each_change_is_kept_for_the_project", () => {
  useNotes.getState().load("p1");
  const id = useNotes.getState().add([10, 20]);
  expect(id).toBe("note:1");
  useNotes.getState().setText(id as string, "the gateway is the only entry");
  useNotes.getState().move(id as string, [30, 40]);
  const saved = JSON.parse(localStorage.getItem("kaanal-notes-p1") ?? "[]");
  expect(saved).toEqual([{ id: "note:1", text: "the gateway is the only entry", at: [30, 40] }]);

  // another project does not see it, and coming back to this one does
  useNotes.getState().load("p2");
  expect(useNotes.getState().notes).toEqual([]);
  useNotes.getState().load("p1");
  expect(useNotes.getState().notes).toHaveLength(1);

  useNotes.getState().remove(id as string);
  expect(JSON.parse(localStorage.getItem("kaanal-notes-p1") ?? "x")).toEqual([]);
});

test("an_id_is_never_reused_while_its_note_is_on_the_page", () => {
  useNotes.getState().load("p");
  const a = useNotes.getState().add([0, 0]) as string;
  const b = useNotes.getState().add([0, 0]) as string;
  useNotes.getState().remove(a);
  const c = useNotes.getState().add([0, 0]) as string;
  expect(new Set([b, c]).size).toBe(2);
  expect(isNoteId(c)).toBe(true);
});

test("the_limits_hold_for_how_many_notes_and_how_long_one_is", () => {
  useNotes.getState().load("p");
  const id = useNotes.getState().add([0, 0]) as string;
  useNotes.getState().setText(id, "x".repeat(MAX_NOTE_CHARS + 50));
  expect(useNotes.getState().notes[0]?.text).toHaveLength(MAX_NOTE_CHARS);
  for (let i = 1; i < MAX_NOTES; i++) useNotes.getState().add([i, i]);
  expect(useNotes.getState().notes).toHaveLength(MAX_NOTES);
  expect(useNotes.getState().add([0, 0])).toBeNull();
});

test("storage_is_not_trusted_a_bad_entry_is_dropped_and_a_bad_file_is_empty", () => {
  expect(parseNotes(null)).toEqual([]);
  expect(parseNotes("not json")).toEqual([]);
  expect(parseNotes('{"a":1}')).toEqual([]);
  const raw = JSON.stringify([
    { id: "note:1", text: "ok", at: [1, 2] },
    { id: "gw", text: "a component id is not a note", at: [1, 2] },
    { id: "note:2", text: 5, at: [1, 2] },
    { id: "note:3", text: "bad place", at: [1] },
    { id: "note:4", text: "not finite", at: [1, null] },
    null,
    { id: "note:5", text: "y".repeat(MAX_NOTE_CHARS + 1), at: [0, 0] },
  ]);
  const notes = parseNotes(raw);
  expect(notes.map((n) => n.id)).toEqual(["note:1", "note:5"]);
  expect(notes[1]?.text).toHaveLength(MAX_NOTE_CHARS);
});

test("blocked_storage_does_not_break_the_notes_they_last_for_the_page", () => {
  vi.stubGlobal("localStorage", {
    getItem: () => {
      throw new Error("blocked");
    },
    setItem: () => {
      throw new Error("blocked");
    },
  });
  useNotes.getState().load("p");
  expect(useNotes.getState().add([1, 1])).toBe("note:1");
  expect(useNotes.getState().notes).toHaveLength(1);
});
