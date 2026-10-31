import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test } from "vite-plus/test";
import { getTheme } from "../theme/theme";
import { ThemeToggle } from "./ThemeToggle";

beforeEach(() => {
  localStorage.clear();
  delete document.documentElement.dataset.theme;
});
afterEach(cleanup);

test("the_button_says_the_current_theme_and_what_a_click_does", () => {
  render(<ThemeToggle />);
  const b = screen.getByRole("button");
  expect(b.getAttribute("aria-label")).toBe("Theme: system. Switch to light");
  expect(b.textContent).toBe(""); // an icon alone, named by its label
});

test("clicking_cycles_system_light_dark_system_and_applies_each", () => {
  render(<ThemeToggle />);
  const b = () => screen.getByRole("button");
  fireEvent.click(b());
  expect(getTheme()).toBe("light");
  expect(b().getAttribute("aria-label")).toBe("Theme: light. Switch to dark");
  fireEvent.click(b());
  expect(getTheme()).toBe("dark");
  expect(document.documentElement.dataset.theme).toBe("dark");
  expect(localStorage.getItem("kaanal-theme")).toBe("dark");
  fireEvent.click(b());
  expect(getTheme()).toBe("system");
  expect(localStorage.getItem("kaanal-theme")).toBeNull();
});

test("it_starts_from_the_theme_already_applied", () => {
  document.documentElement.dataset.theme = "dark";
  render(<ThemeToggle />);
  expect(screen.getByRole("button").getAttribute("aria-label")).toBe(
    "Theme: dark. Switch to system",
  );
});

test("the_target_is_at_least_24_px", () => {
  render(<ThemeToggle />);
  expect(screen.getByRole("button").className).toMatch(/\bh-8\b/);
});
