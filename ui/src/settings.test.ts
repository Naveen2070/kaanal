// The settings that outlive a page load. Only the minimum load time so far; a settings menu will
// call the setters. Storage can be blocked: reads fall back to the default, writes never throw.

import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";
import { DEFAULT_MIN_LOAD_MS, getMinLoadMs, setMinLoadMs } from "./settings";

beforeEach(() => localStorage.clear());
afterEach(() => vi.restoreAllMocks());

describe("minimum load time", () => {
  it("defaults_to_ten_seconds", () => {
    expect(DEFAULT_MIN_LOAD_MS).toBe(10_000);
    expect(getMinLoadMs()).toBe(10_000);
  });

  it("a_saved_value_wins_and_zero_turns_the_hold_off", () => {
    setMinLoadMs(3000);
    expect(getMinLoadMs()).toBe(3000);
    setMinLoadMs(0);
    expect(getMinLoadMs()).toBe(0);
  });

  it("junk_and_negative_values_fall_back_and_huge_ones_are_capped", () => {
    localStorage.setItem("kaanal-min-load-ms", "soon");
    expect(getMinLoadMs()).toBe(DEFAULT_MIN_LOAD_MS);
    localStorage.setItem("kaanal-min-load-ms", "-5");
    expect(getMinLoadMs()).toBe(DEFAULT_MIN_LOAD_MS);
    setMinLoadMs(10 ** 9);
    expect(getMinLoadMs()).toBe(60_000);
  });

  it("blocked_storage_is_not_an_error", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new DOMException("blocked", "SecurityError");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new DOMException("blocked", "SecurityError");
    });
    expect(getMinLoadMs()).toBe(DEFAULT_MIN_LOAD_MS);
    expect(() => setMinLoadMs(5000)).not.toThrow();
  });
});
