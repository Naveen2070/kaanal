import { afterEach, expect, test, vi } from "vite-plus/test";
import { useDraft } from "../data/store/draft";
import { useHistory } from "../data/store/history";
import { notesOf, seedDraft } from "../test/notesDraft";
import {
  addNoteAt,
  importBrowserNotes,
  isNoteId,
  MAX_NOTE_CHARS,
  MAX_NOTES,
  moveNote,
  parseNotes,
  removeNote,
  setNoteText,
} from "./notes";

import { useToasts } from "../shell/toast/toasts";
import type { Note } from "../data/api/types";

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
});

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

const browser = [{ id: "note:1", text: "from the browser", at: [5, 6] }];
let n = 0;
/** A project id nobody imported yet (the once-per-project memory lives for the page). */
const fresh = async (notes?: Note[], file: Note[] = []) => {
  const sent = await seedDraft(file);
  const project = `p${n++}`;
  useDraft.setState((s) => ({
    view: s.view && {
      ...s.view,
      project_id: project,
      canvas_notes: notes === undefined ? undefined : file,
    },
  }));
  return { sent, project };
};
const keyOf = (p: string) => `kaanal-notes-${p}`;
const view = () => useDraft.getState().view;

test("imports_browser_notes_when_the_file_has_none", async () => {
  const { sent, project } = await fresh([]);
  localStorage.setItem(keyOf(project), JSON.stringify(browser));
  importBrowserNotes(view());
  await settle();
  expect(sent).toHaveLength(1);
  expect(sent[0]?.body).toEqual({ canvas_notes: browser });
  expect(notesOf()).toEqual(browser);
});

test("file_notes_win_over_the_browser", async () => {
  const file = [{ id: "note:9", text: "from the file", at: [0, 0] as [number, number] }];
  const { sent, project } = await fresh(file, file);
  localStorage.setItem(keyOf(project), JSON.stringify(browser));
  importBrowserNotes(view());
  await settle();
  expect(sent).toEqual([]);
  expect(notesOf()).toEqual(file);
});

test("an_import_is_sent_once_and_the_flag_waits_for_a_save", async () => {
  const { sent, project } = await fresh([]);
  localStorage.setItem(keyOf(project), JSON.stringify(browser));
  importBrowserNotes(view());
  await settle();
  const flag = `kaanal-notes-imported-${project}`;
  const dirty = (layout_dirty: boolean) =>
    useDraft.setState((s) => ({
      view: s.view && { ...s.view, project_id: project, layout_dirty, canvas_notes: [] },
    }));
  dirty(true); // the answer to the PUT: not saved yet
  importBrowserNotes(view());
  expect(localStorage.getItem(flag)).toBeNull(); // a restart now would try again
  dirty(false); // saved (even if the user deleted the notes first)
  importBrowserNotes(view());
  await settle();
  expect(sent).toHaveLength(1);
  expect(localStorage.getItem(flag)).toBe("1");
});

test("an_import_that_was_never_saved_is_tried_again_on_a_new_boot", async () => {
  const { sent, project } = await fresh([]);
  localStorage.setItem(keyOf(project), JSON.stringify(browser));
  importBrowserNotes(view());
  await settle();
  useDraft.setState((s) => ({
    view: s.view && {
      ...s.view,
      project_id: project,
      boot: "b2",
      layout_dirty: false,
      canvas_notes: [],
    },
  })); // the server restarted before a Save
  importBrowserNotes(view());
  await settle();
  expect(sent).toHaveLength(2);
});

test("a_refused_import_is_not_flagged_and_can_be_tried_again", async () => {
  const { project } = await fresh([]);
  localStorage.setItem(keyOf(project), JSON.stringify(browser));
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => new Response('{"error":"no"}', { status: 400 })),
  );
  importBrowserNotes(view());
  await settle();
  await Promise.resolve();
  expect(localStorage.getItem(`kaanal-notes-imported-${project}`)).toBeNull();
});

test("the_browser_list_is_filtered_by_the_servers_rules", () => {
  const raw = JSON.stringify([
    { id: "note:1", text: "a", at: [0, 0] },
    { id: "note:1", text: "duplicate id", at: [0, 0] },
    { id: `note:${"x".repeat(60)}`, text: "id over 64 bytes", at: [0, 0] },
    { id: "note:2", text: "too far", at: [2e6, 0] },
    { id: "note:3", text: "edge", at: [1e6, -1e6] },
  ]);
  expect(parseNotes(raw).map((n) => n.id)).toEqual(["note:1", "note:3"]);
});

test("a_later_session_does_not_bring_back_notes_the_user_deleted", async () => {
  const { sent, project } = await fresh([]);
  localStorage.setItem(keyOf(project), JSON.stringify(browser));
  localStorage.setItem(`kaanal-notes-imported-${project}`, "1"); // an earlier session imported
  importBrowserNotes(view());
  await settle();
  expect(sent).toEqual([]);
});

test("import_ignores_malformed_or_foreign_project_storage", async () => {
  const { sent, project } = await fresh([]);
  localStorage.setItem(keyOf(project), "not json");
  localStorage.setItem("kaanal-notes-other", JSON.stringify(browser));
  importBrowserNotes(view());
  await settle();
  expect(sent).toEqual([]);
});

test("the_browser_copy_is_never_deleted", async () => {
  const { project } = await fresh([]);
  const raw = JSON.stringify(browser);
  localStorage.setItem(keyOf(project), raw);
  importBrowserNotes(view());
  await settle();
  expect(localStorage.getItem(keyOf(project))).toBe(raw);
});

test("blocked_storage_imports_nothing", async () => {
  const { sent } = await fresh([]);
  vi.stubGlobal("localStorage", {
    getItem: () => {
      throw new Error("blocked");
    },
    setItem: () => {
      throw new Error("blocked");
    },
  });
  importBrowserNotes(view());
  await settle();
  expect(sent).toEqual([]);
});

test("an_older_server_without_notes_imports_nothing_and_refuses_an_added_note_with_a_notice", async () => {
  const { sent, project } = await fresh(undefined);
  localStorage.setItem(keyOf(project), JSON.stringify(browser));
  importBrowserNotes(view());
  expect(addNoteAt([0, 0])).toBeNull();
  await settle();
  expect(sent).toEqual([]); // nothing was sent
  expect(useToasts.getState().toasts.some((t) => /too old/.test(t.text))).toBe(true);
});
