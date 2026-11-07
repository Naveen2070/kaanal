import { beforeEach, expect, test } from "vite-plus/test";
import { useGuide } from "./store";

beforeEach(() => useGuide.setState({ open: false, chapter: "components", entry: null }));

test("a_string_still_opens_a_chapter", () => {
  useGuide.getState().show("screen");
  expect(useGuide.getState()).toMatchObject({ open: true, chapter: "screen", entry: null });
});

test("no_argument_opens_where_it_was_left", () => {
  useGuide.setState({ chapter: "wiring" });
  useGuide.getState().show();
  expect(useGuide.getState()).toMatchObject({ open: true, chapter: "wiring", entry: null });
});

test("an_entry_is_remembered_for_the_dialog_to_find", () => {
  useGuide.getState().show({ entry: "p99" });
  expect(useGuide.getState()).toMatchObject({ open: true, chapter: "components", entry: "p99" });
});

test("an_entry_with_a_chapter_keeps_both", () => {
  useGuide.getState().show({ chapter: "terms", entry: "p99" });
  expect(useGuide.getState()).toMatchObject({ chapter: "terms", entry: "p99" });
});

test("choosing_a_chapter_by_hand_forgets_the_entry", () => {
  useGuide.getState().show({ entry: "p99" });
  useGuide.getState().setChapter("components");
  expect(useGuide.getState().entry).toBeNull();
});
