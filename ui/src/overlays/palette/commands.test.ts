import { expect, test, vi } from "vite-plus/test";
import { commands, filterCommands } from "./commands";

const ctx = (note?: () => void) => ({
  add: () => {},
  run: () => {},
  press: () => {},
  tour: () => {},
  fit: () => {},
  traffic: () => {},
  note,
});

test("add_a_note_is_a_command_and_finds_by_the_word_note", () => {
  const note = vi.fn();
  const found = filterCommands(commands(ctx(note)), "note");
  expect(found.map((c) => c.id)).toEqual(["note"]);
  found[0]?.run();
  expect(note).toHaveBeenCalledTimes(1);
  // without the callback the command is harmless
  expect(() =>
    commands(ctx())
      .find((c) => c.id === "note")
      ?.run(),
  ).not.toThrow();
});
