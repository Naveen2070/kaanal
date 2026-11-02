import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { mockFetch } from "../../test/fetch";
import { SaveFlow } from "./SaveFlow";

function view(over: Partial<DraftView> = {}): DraftView {
  return {
    text: "version: 1\nnodes: {}\n",
    dirty: false,
    generation: 1,
    lost: [],
    rev: 0,
    boot: "b",
    doc: { nodes: {}, traffic: { peak: {} } },
    doc_error: null,
    project_id: "p",
    project_name: null,
    kind_defaults: {},
    layout: {},
    layout_note: null,
    layout_dirty: false,
    saved: true,
    ...over,
  };
}

async function show(v: DraftView = view()) {
  mockFetch([{ json: v }]);
  await useDraft.getState().load();
  render(<SaveFlow />);
}

const saved = {
  ok: true,
  written: ["kaanal.yaml"],
  removed: [],
  layout_written: true,
  layout_note: null,
};
const valid = { ok: true, diagnostics: [], warnings: [], flagged: [] };

beforeEach(() => useDraft.setState({ view: null, error: null, restarted: false }));
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

test("the_applied_and_dirty_indicators_follow_the_draft_view", async () => {
  await show(view({ dirty: false }));
  expect(screen.getByText("Draft: applied · saved")).toBeTruthy();
  act(() => useDraft.setState({ view: view({ dirty: true, saved: false }) }));
  expect(screen.getByText("Draft: not applied · not saved")).toBeTruthy();
  act(() => useDraft.setState({ view: view({ dirty: false, saved: false }) }));
  expect(screen.getByText("Draft: applied · not saved")).toBeTruthy();
});

test("save_checks_the_draft_then_writes_the_file_and_says_what_was_written", async () => {
  await show();
  const sent = mockFetch([{ json: valid }, { json: saved }, { json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByRole("dialog", { name: "Saved" })).toBeTruthy();
  expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual([
    "POST /__kaanal/draft/validate",
    "POST /__kaanal/draft/save",
    "GET /__kaanal/draft",
  ]);
  expect(sent[1]?.body).toEqual({});
  expect(screen.getByText(/Written: kaanal.yaml/)).toBeTruthy();
  expect(screen.getByText(/Positions saved beside it/)).toBeTruthy();
});

test("a_draft_that_would_not_start_asks_before_it_is_written", async () => {
  await show();
  const sent = mockFetch([
    {
      json: {
        ok: false,
        diagnostics: [{ message: "node 'db': unknown field 'proc'" }],
        warnings: [],
        flagged: [{ file: "scenarios/a.yaml", scenario: "a", message: "names node 'old'" }],
      },
    },
  ]);
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByText("node 'db': unknown field 'proc'")).toBeTruthy();
  expect(screen.getByText(/scenario file scenarios\/a.yaml/)).toBeTruthy();
  expect(sent).toHaveLength(1); // nothing written yet
  const more = mockFetch([{ json: saved }, { json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Save anyway" }));
  await vi.waitFor(() => expect(more.length).toBeGreaterThan(0));
  expect(more[0]?.url).toBe("/__kaanal/draft/save"); // no second check
  expect(await screen.findByRole("dialog", { name: "Saved" })).toBeTruthy();
});

test("save_asks_before_losing_comments_and_sends_the_confirmation", async () => {
  await show();
  mockFetch([
    { json: valid },
    { status: 409, json: { error: "the draft lost: a comment", lost: ["a comment above db"] } },
  ]);
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByText("a comment above db")).toBeTruthy();
  const sent = mockFetch([{ json: saved }, { json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Save anyway" }));
  await vi.waitFor(() => expect(sent.length).toBeGreaterThan(0));
  expect(sent[0]?.body).toEqual({ confirm_loss: true });
  expect(await screen.findByRole("dialog", { name: "Saved" })).toBeTruthy();
});

test("cancelling_the_loss_prompt_writes_nothing", async () => {
  await show();
  mockFetch([{ json: valid }, { status: 409, json: { error: "lost", lost: ["x"] } }]);
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await screen.findByText("x");
  const sent = mockFetch([{ json: saved }]);
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(sent).toHaveLength(0);
});

async function conflict() {
  await show();
  mockFetch([
    { json: valid },
    {
      status: 409,
      json: {
        error: "the file changed on disk since Studio read it; nothing was written",
        conflict: true,
      },
    },
  ]);
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await screen.findByText(/the file changed on disk since Studio read it/);
}

test("a_conflict_says_the_truth_about_how_to_recover_and_copies_the_draft", async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  vi.stubGlobal("navigator", { clipboard: { writeText } });
  await conflict();
  // it must not promise that reopening the page re-reads the file: only a restart does
  expect(screen.getByText(/restart/).textContent).toMatch(/lost on restart/);
  fireEvent.click(screen.getByRole("button", { name: "Copy my draft" }));
  expect(writeText).toHaveBeenCalledWith("version: 1\nnodes: {}\n");
  expect(await screen.findByRole("button", { name: "Copied" })).toBeTruthy();
});

test("when_copying_is_blocked_the_draft_is_shown_to_select_by_hand", async () => {
  vi.stubGlobal("navigator", {});
  await conflict();
  fireEvent.click(screen.getByRole("button", { name: "Copy my draft" }));
  const box = (await screen.findByLabelText(/Copying was blocked/)) as HTMLTextAreaElement;
  expect(box.value).toBe("version: 1\nnodes: {}\n");
  expect(box.readOnly).toBe(true);
});

test("a_save_error_keeps_the_draft_and_shows_the_message", async () => {
  await show();
  const before = useDraft.getState().view;
  mockFetch([{ json: valid }, { status: 500, json: { error: "disk full" } }]);
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect((await screen.findByRole("alert")).textContent).toBe("disk full");
  expect(screen.getByText("The draft is unchanged.")).toBeTruthy();
  expect(useDraft.getState().view).toBe(before);
});

test("apply_shows_what_was_kept_and_dropped", async () => {
  await show();
  const sent = mockFetch([
    {
      json: {
        ok: true,
        generation: 2,
        kept: { live_chaos: 1, active_scenarios: ["s1"], pins: 0 },
        dropped: { live_chaos: ["node 'db'"], pins: [] },
        flagged: [],
      },
    },
    { json: view({ generation: 2 }) },
  ]);
  fireEvent.click(screen.getByRole("button", { name: "Apply" }));
  expect(await screen.findByRole("dialog", { name: "Applied" })).toBeTruthy();
  expect(sent[0]).toMatchObject({ url: "/__kaanal/draft/apply", method: "POST" });
  expect(screen.getByText(/generation 2/)).toBeTruthy();
  expect(screen.getByText("Kept: 1 live chaos, 1 active scenarios")).toBeTruthy();
  expect(screen.getByText("Dropped: live chaos: node 'db'")).toBeTruthy();
});

test("a_refused_apply_names_the_problem_in_the_draft", async () => {
  await show();
  mockFetch([
    {
      status: 400,
      json: {
        error: "the draft does not build",
        diagnostics: [{ message: "edge #2: 'app' cannot call itself" }],
      },
    },
  ]);
  fireEvent.click(screen.getByRole("button", { name: "Apply" }));
  expect((await screen.findByRole("alert")).textContent).toBe("edge #2: 'app' cannot call itself");
});

test("apply_and_save_wait_for_an_edit_still_on_its_way", async () => {
  await show();
  // an edit whose answer is slow: the request for Apply must not leave before it is answered
  const sent = mockFetch([
    { json: view({ rev: 1 }), delayMs: 40 },
    {
      json: { ok: true, generation: 2, kept: {}, dropped: {}, flagged: [] },
    },
    { json: view({ rev: 1, generation: 2 }) },
  ]);
  void useDraft.getState().setLayout({ gw: [1, 2] });
  fireEvent.click(screen.getByRole("button", { name: "Apply" }));
  await screen.findByRole("dialog", { name: "Applied" });
  expect(sent.map((s) => `${s.method} ${s.url}`).slice(0, 2)).toEqual([
    "PUT /__kaanal/draft",
    "POST /__kaanal/draft/apply",
  ]);
});

test("the_buttons_are_off_when_the_draft_is_not_a_config_and_while_one_is_running", async () => {
  await show(view({ doc: null }));
  expect((screen.getByRole("button", { name: "Apply" }) as HTMLButtonElement).disabled).toBe(true);
  expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true);
  cleanup();
  await show();
  mockFetch([{ json: valid, delayMs: 30 }, { json: saved }, { json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect((screen.getByRole("button", { name: "Apply" }) as HTMLButtonElement).disabled).toBe(true);
  await screen.findByRole("dialog", { name: "Saved" });
});

test("closing_a_dialog_gives_focus_back_to_the_button_that_opened_it", async () => {
  await show();
  mockFetch([{ json: valid }, { json: saved }, { json: view() }]);
  const save = screen.getByRole("button", { name: "Save" });
  save.focus();
  fireEvent.click(save);
  await screen.findByRole("dialog", { name: "Saved" });
  fireEvent.click(screen.getByRole("button", { name: "Close" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(document.activeElement).toBe(save);
});

// the run loop cannot apply or save: its dependencies do not even include those calls
const loopSources = import.meta.glob("../../data/store/loop*.ts", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

test("apply_is_never_called_by_the_run_loop", () => {
  const files = Object.entries(loopSources).filter(([f]) => !f.endsWith(".test.ts"));
  expect(files.length).toBeGreaterThanOrEqual(2);
  for (const [f, src] of files) {
    expect(src, f).not.toMatch(/\.apply\(|\/draft\/apply|\.save\(|\/draft\/save/);
  }
});
