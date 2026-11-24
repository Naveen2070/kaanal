import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { useDraft } from "../data/store/draft";
import { notesOf, seedDraft } from "../test/notesDraft";
import { NoteCard } from "./NoteCard";

vi.mock("@xyflow/react", () => ({}));

const props = (text: string) =>
  ({ id: "note:1", data: { text } }) as unknown as Parameters<typeof NoteCard>[0];

beforeEach(async () => {
  await seedDraft([{ id: "note:1", text: "", at: [0, 0] }]);
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});
const settle = () => useDraft.getState().settled();

test("typing_is_kept_when_the_box_loses_focus_not_on_every_key", async () => {
  render(<NoteCard {...props("")} />);
  const box = screen.getByLabelText("Note") as HTMLTextAreaElement;
  fireEvent.change(box, { target: { value: "queue sits before the db" } });
  expect(notesOf()[0]?.text).toBe(""); // still typing
  fireEvent.blur(box);
  await settle();
  expect(notesOf()[0]?.text).toBe("queue sits before the db");
});

test("the_box_is_marked_so_selecting_text_does_not_drag_the_card_or_zoom_the_canvas", () => {
  render(<NoteCard {...props("hello")} />);
  const box = screen.getByLabelText("Note");
  expect(box.className).toMatch(/\bnodrag\b/);
  expect(box.className).toMatch(/\bnowheel\b/);
  expect((box as HTMLTextAreaElement).value).toBe("hello");
});

test("the_delete_button_removes_the_note", async () => {
  render(<NoteCard {...props("bye")} />);
  fireEvent.click(screen.getByRole("button", { name: "Delete this note" }));
  await settle();
  expect(notesOf()).toEqual([]);
});
