import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test } from "vite-plus/test";
import { getTheme } from "../../look/theme/theme";
import { ThemeToggle } from "./ThemeToggle";

beforeEach(() => {
  localStorage.clear();
  delete document.documentElement.dataset.theme;
});
afterEach(cleanup);

test("one_button_names_the_current_theme_and_the_next", () => {
  render(<ThemeToggle />);
  expect(screen.getAllByRole("button")).toHaveLength(1);
  expect(
    screen.getByRole("button", { name: "Theme: the system theme. Switch to light" }),
  ).toBeTruthy();
});

test("a_click_moves_to_the_next_theme_and_remembers_it", () => {
  render(<ThemeToggle />);
  fireEvent.click(screen.getByRole("button"));
  expect(getTheme()).toBe("light");
  fireEvent.click(screen.getByRole("button", { name: "Theme: light. Switch to dark" }));
  expect(document.documentElement.dataset.theme).toBe("dark");
  expect(localStorage.getItem("kaanal-theme")).toBe("dark");
  fireEvent.click(screen.getByRole("button", { name: "Theme: dark. Switch to the system theme" }));
  expect(getTheme()).toBe("system");
  expect(localStorage.getItem("kaanal-theme")).toBeNull();
});

test("it_starts_from_the_theme_already_applied", () => {
  document.documentElement.dataset.theme = "dark";
  render(<ThemeToggle />);
  expect(screen.getByRole("button").getAttribute("aria-label")).toMatch(/^Theme: dark/);
});

test("the_command_palette_presses_the_same_button", () => {
  render(<ThemeToggle />);
  document.querySelector<HTMLElement>('[data-command="theme"]')?.click();
  expect(getTheme()).toBe("light");
});

test("the_target_is_at_least_24_px", () => {
  render(<ThemeToggle />);
  expect(screen.getByRole("button").className).toMatch(/\bsize-8\b/);
});
