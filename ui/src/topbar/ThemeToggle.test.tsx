import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test } from "vite-plus/test";
import { getTheme } from "../theme/theme";
import { ThemeToggle } from "./ThemeToggle";

beforeEach(() => {
  localStorage.clear();
  delete document.documentElement.dataset.theme;
});
afterEach(cleanup);

const pressed = (name: string) =>
  screen.getByRole("button", { name }).getAttribute("aria-pressed") === "true";

test("three_buttons_show_the_current_choice_as_pressed", () => {
  render(<ThemeToggle />);
  expect(screen.getAllByRole("button")).toHaveLength(3);
  expect(pressed("Follow the system theme")).toBe(true);
  expect(pressed("Dark theme")).toBe(false);
});

test("a_click_applies_and_remembers_the_choice", () => {
  render(<ThemeToggle />);
  fireEvent.click(screen.getByRole("button", { name: "Light theme" }));
  expect(getTheme()).toBe("light");
  fireEvent.click(screen.getByRole("button", { name: "Dark theme" }));
  expect(document.documentElement.dataset.theme).toBe("dark");
  expect(localStorage.getItem("kaanal-theme")).toBe("dark");
  expect(pressed("Dark theme")).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: "Follow the system theme" }));
  expect(getTheme()).toBe("system");
  expect(localStorage.getItem("kaanal-theme")).toBeNull();
});

test("it_starts_from_the_theme_already_applied", () => {
  document.documentElement.dataset.theme = "dark";
  render(<ThemeToggle />);
  expect(pressed("Dark theme")).toBe(true);
});

test("the_command_palette_presses_the_next_theme_in_turn", () => {
  render(<ThemeToggle />);
  const next = document.querySelector<HTMLElement>('[data-command="theme"]');
  expect(next?.getAttribute("aria-label")).toBe("Light theme");
  next?.click();
  expect(getTheme()).toBe("light");
});

test("each_target_is_at_least_24_px", () => {
  render(<ThemeToggle />);
  for (const b of screen.getAllByRole("button")) expect(b.className).toMatch(/\bh-7\b/);
});
