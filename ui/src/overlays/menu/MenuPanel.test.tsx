import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { Check } from "lucide-react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../../data/api/types";
import { useGuide } from "../guide/store";
import { useDraft } from "../../data/store/draft";
import { TOKEN_KEY } from "../../token";
import { MenuPanel, type MenuItem } from "./MenuPanel";

const TOKEN = "s3cret-token-value";

function view(over: Partial<DraftView> = {}): DraftView {
  return {
    text: "",
    dirty: true,
    generation: 1,
    lost: [],
    rev: 0,
    boot: "b",
    doc: { nodes: {} },
    doc_error: null,
    project_id: "p-1234",
    project_name: "shop",
    kind_defaults: {},
    layout: {},
    layout_note: null,
    layout_dirty: false,
    saved: false,
    ...over,
  };
}

beforeEach(() => {
  sessionStorage.setItem(TOKEN_KEY, TOKEN);
  useDraft.setState({ view: view() });
  useGuide.setState({ open: false, chapter: "components" });
});
afterEach(() => {
  cleanup();
  sessionStorage.clear();
  vi.unstubAllGlobals();
});

const panel = (onClose = vi.fn()) => render(<MenuPanel top={10} right={10} onClose={onClose} />);

test("lists_the_glossary_the_tour_and_the_profile", () => {
  panel();
  const items = screen.getAllByRole("button").map((b) => b.textContent);
  expect(items[0]).toContain("Glossary");
  expect(items[1]).toContain("Show the tour");
  expect(items[2]).toContain("Profile");
});

test("the_glossary_item_opens_the_glossary_and_closes_the_menu", () => {
  const onClose = vi.fn();
  panel(onClose);
  fireEvent.click(screen.getByRole("button", { name: /Glossary/ }));
  expect(useGuide.getState().open).toBe(true);
  expect(onClose).toHaveBeenCalled();
});

test("the_profile_item_opens_in_place_with_a_way_back", () => {
  panel();
  fireEvent.click(screen.getByRole("button", { name: /Profile/ }));
  expect(screen.getByRole("heading", { name: "Profile" })).toBeTruthy();
  expect(screen.getByText("shop")).toBeTruthy();
  expect(screen.getByText("p-1234")).toBeTruthy();
  expect(screen.getByText(location.origin)).toBeTruthy();
  expect(screen.getByText("not applied · not saved")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Menu" }));
  expect(screen.getByRole("button", { name: /Glossary/ })).toBeTruthy();
});

test("the_token_is_masked_until_shown_and_can_be_copied", async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  vi.stubGlobal("navigator", { clipboard: { writeText } });
  panel();
  fireEvent.click(screen.getByRole("button", { name: /Profile/ }));
  expect(document.body.textContent).not.toContain(TOKEN);
  fireEvent.click(screen.getByRole("button", { name: "Show" }));
  expect(screen.getByText(TOKEN)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Hide" }));
  expect(document.body.textContent).not.toContain(TOKEN);
  fireEvent.click(screen.getByRole("button", { name: "Copy" }));
  expect(writeText).toHaveBeenCalledWith(TOKEN);
  expect(await screen.findByRole("button", { name: "Copied" })).toBeTruthy();
});

test("with_no_token_the_profile_says_missing_and_offers_no_copy", () => {
  sessionStorage.clear();
  useDraft.setState({
    view: view({ project_name: null, project_id: "none", dirty: false, saved: true }),
  });
  panel();
  fireEvent.click(screen.getByRole("button", { name: /Profile/ }));
  expect(screen.getByText("missing")).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Copy" })).toBeNull();
  expect(screen.getByText("No project file yet")).toBeTruthy();
  expect(screen.getByText("applied · saved")).toBeTruthy();
});

test("a_new_menu_is_one_more_entry", () => {
  const run = vi.fn();
  const extra: MenuItem[] = [{ id: "x", label: "Settings", hint: "More later", icon: Check, run }];
  const onClose = vi.fn();
  render(<MenuPanel top={0} right={0} onClose={onClose} items={extra} />);
  fireEvent.click(screen.getByRole("button", { name: /Settings/ }));
  expect(run).toHaveBeenCalled();
  expect(onClose).toHaveBeenCalled();
});
