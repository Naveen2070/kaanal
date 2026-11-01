import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vite-plus/test";
import { CommandPalette } from "./CommandPalette";
import { commands, filterCommands } from "./commands";
import { PaletteHost } from "./PaletteHost";

afterEach(cleanup);

const ctx = () => ({
  add: vi.fn(),
  run: vi.fn(),
  press: vi.fn(),
  tour: vi.fn(),
  fit: vi.fn(),
});

test("commands_cover_add_run_apply_save_theme_tour_fit", () => {
  const ids = commands(ctx()).map((c) => c.id);
  for (const id of ["add-service", "run", "apply", "save", "theme", "tour", "fit"])
    expect(ids).toContain(id);
});

test("typing_filters_and_enter_runs_the_active_option", () => {
  const c = ctx();
  const onClose = vi.fn();
  render(<CommandPalette all={commands(c)} onClose={onClose} />);
  const input = screen.getByRole("combobox");
  fireEvent.change(input, { target: { value: "add serv" } });
  const options = screen.getAllByRole("option");
  expect(options.map((o) => o.textContent)).toEqual(["Add a service"]);
  expect(input.getAttribute("aria-activedescendant")).toBe(options[0]?.id);
  fireEvent.keyDown(input, { key: "Enter" });
  expect(c.add).toHaveBeenCalledWith("service");
  expect(onClose).toHaveBeenCalled();
});

test("arrow_keys_move_the_active_option", () => {
  render(<CommandPalette all={commands(ctx())} onClose={() => {}} />);
  const input = screen.getByRole("combobox");
  const first = input.getAttribute("aria-activedescendant");
  fireEvent.keyDown(input, { key: "ArrowDown" });
  expect(input.getAttribute("aria-activedescendant")).not.toBe(first);
  fireEvent.keyDown(input, { key: "ArrowUp" });
  expect(input.getAttribute("aria-activedescendant")).toBe(first);
});

test("filter_matches_every_word_in_any_order", () => {
  const all = commands(ctx());
  expect(filterCommands(all, "the replay").map((c) => c.id)).toEqual(["tour"]);
  expect(filterCommands(all, "zzz")).toEqual([]);
  expect(filterCommands(all, "")).toHaveLength(all.length);
});

test("ctrl_k_opens_a_modal_dialog_and_escape_closes_with_focus_back", async () => {
  const opener = document.createElement("button");
  document.body.append(opener);
  opener.focus();
  render(<PaletteHost />);
  expect(screen.queryByRole("dialog")).toBeNull();
  await act(async () => void fireEvent.keyDown(window, { key: "k", ctrlKey: true }));
  const dialog = await screen.findByRole("dialog", { name: "Command palette" });
  expect(dialog.getAttribute("aria-modal")).toBe("true");
  await act(async () => void fireEvent(dialog, new Event("cancel", { cancelable: true })));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(document.activeElement).toBe(opener);
  opener.remove();
});
