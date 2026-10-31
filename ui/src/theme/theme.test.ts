// The theme choice: the saved one wins over the system setting, and blocked storage is not an error.

import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";
import { getTheme, initTheme, setTheme } from "./theme";

const root = document.documentElement;

beforeEach(() => {
  localStorage.clear();
  delete root.dataset.theme;
});
afterEach(() => vi.restoreAllMocks());

describe("theme", () => {
  it("defaults to the system setting", () => {
    initTheme();
    expect(getTheme()).toBe("system");
    expect(root.dataset.theme).toBeUndefined();
  });

  it("saved_theme_wins_over_system", () => {
    vi.stubGlobal("matchMedia", () => ({ matches: false })); // the system says light
    localStorage.setItem("kaanal-theme", "dark");
    initTheme();
    expect(getTheme()).toBe("dark");
    expect(root.dataset.theme).toBe("dark");
    vi.unstubAllGlobals();
  });

  it("setTheme_persists_and_system_clears_the_choice", () => {
    setTheme("light");
    expect(localStorage.getItem("kaanal-theme")).toBe("light");
    expect(root.dataset.theme).toBe("light");
    setTheme("system");
    expect(localStorage.getItem("kaanal-theme")).toBeNull();
    expect(root.dataset.theme).toBeUndefined();
    expect(getTheme()).toBe("system");
  });

  it("ignores a saved value it does not know", () => {
    localStorage.setItem("kaanal-theme", "purple");
    initTheme();
    expect(getTheme()).toBe("system");
  });

  it("setTheme_survives_blocked_storage", () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new DOMException("blocked", "SecurityError");
    });
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new DOMException("blocked", "SecurityError");
    });
    expect(() => setTheme("dark")).not.toThrow();
    expect(getTheme()).toBe("dark"); // this page still follows the choice
    expect(() => initTheme()).not.toThrow();
  });
});
