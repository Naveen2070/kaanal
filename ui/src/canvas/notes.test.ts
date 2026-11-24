import { afterEach, expect, test, vi } from "vite-plus/test";
import { useDraft } from "../data/store/draft";
import { useHistory } from "../data/store/history";
import { notesOf, seedDraft } from "../test/notesDraft";
import {
  addNoteAt,
  isNoteId,
  MAX_NOTE_CHARS,
  MAX_NOTES,
  moveNote,
  parseNotes,
  removeNote,
  setNoteText,
} from "./notes";

afterEach(() => vi.unstubAllGlobals());

const settle = () => useDraft.getState().settled();

test("a_note_is_added_edited_moved_and_removed_and_each_change_sends_the_whole_list", async () => {
  const sent = await seedDraft();
  const id = addNoteAt([10, 20]) as string;
  expect(id).toBe("note:1");
  setNoteText(id, "the gateway is the only entry");
  moveNote(id, [30, 40]);
  await settle();
  expect(sent.map((s) => s.method)).toEqual(["PUT", "PUT", "PUT"]);
  expect(sent[2]?.body).toEqual({
    canvas_notes: [{ id: "note:1", text: "the gateway is the only entry", at: [30, 40] }],
  });
  expect(notesOf()).toHaveLength(1);
  removeNote(id);
  await settle();
  expect(sent[3]?.body).toEqual({ canvas_notes: [] });
  expect(notesOf()).toEqual([]);
});

test("edits_made_before_the_first_answer_build_on_each_other", async () => {
  const sent = await seedDraft();
  addNoteAt([0, 0]);
  addNoteAt([1, 1]); // no await: the first answer has not come yet
  expect(notesOf().map((n) => n.id)).toEqual(["note:1", "note:2"]); // shown at once
  await settle();
  expect(sent[1]?.body?.canvas_notes?.map((n) => n.id)).toEqual(["note:1", "note:2"]);
});

test("notes_are_not_an_undoable_edit", async () => {
  await seedDraft();
  useHistory.setState({ past: [], future: [], note: null });
  addNoteAt([0, 0]);
  await settle();
  expect(useHistory.getState().past).toEqual([]);
});

test("an_id_is_never_reused_while_its_note_is_on_the_page", async () => {
  await seedDraft();
  const a = addNoteAt([0, 0]) as string;
  const b = addNoteAt([0, 0]) as string;
  await settle();
  removeNote(a);
  const c = addNoteAt([0, 0]) as string;
  await settle();
  expect(new Set([b, c]).size).toBe(2);
  expect(isNoteId(c)).toBe(true);
});

test("the_limits_hold_for_how_many_notes_and_how_long_one_is", async () => {
  const sent = await seedDraft();
  const id = addNoteAt([0, 0]) as string;
  setNoteText(id, "x".repeat(MAX_NOTE_CHARS + 50));
  await settle();
  expect(notesOf()[0]?.text).toHaveLength(MAX_NOTE_CHARS);
  for (let i = 1; i < MAX_NOTES; i++) addNoteAt([i, i]);
  await settle();
  expect(notesOf()).toHaveLength(MAX_NOTES);
  const requests = sent.length;
  expect(addNoteAt([0, 0])).toBeNull();
  await settle();
  expect(sent).toHaveLength(requests); // nothing was sent for the refused one
});

test("a_refused_edit_is_taken_back_and_says_why", async () => {
  await seedDraft([{ id: "note:1", text: "kept", at: [0, 0] }], { refuse: true });
  setNoteText("note:1", "changed");
  await settle();
  expect(notesOf()[0]?.text).toBe("kept");
  expect(useDraft.getState().error).toMatch(/refused/);
});

test("a_server_that_has_no_notes_shows_none", async () => {
  await seedDraft();
  useDraft.setState((s) => ({ view: s.view && { ...s.view, canvas_notes: undefined } }));
  expect(notesOf()).toEqual([]);
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
