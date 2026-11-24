import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../../data/api/types";
import type { PlanRecord } from "../../data/api/types.plan";
import { useDraft } from "../../data/store/draft";
import { useMode } from "../../data/store/mode";
import { usePlan } from "../../data/store/plan";
import { useRun } from "../../data/store/run";
import { useToasts } from "../../shell/toast/toasts";
import { expectNoViolations } from "../../test/axe";
import { mockFetch } from "../../test/fetch";
import { planCannotHold, planCheapest, planDone, planRunning } from "../../test/planRecord";
import { TOKEN_KEY } from "../../token";
import { DEFAULT_FORM } from "./planMath";
import { PlanHost } from "./PlanHost";

const view = (over: Partial<DraftView> = {}): DraftView => ({
  text: "version: 1\nnodes: {}\n",
  dirty: false,
  generation: 2,
  lost: [],
  rev: 0,
  boot: "b",
  doc: {
    nodes: { app: { type: "service" }, db: { type: "database" }, cache: { type: "cache" } },
    edges: [],
    traffic: { peak: {} },
  },
  doc_error: null,
  project_id: "p",
  project_name: null,
  kind_defaults: {},
  layout: {},
  layout_note: null,
  layout_dirty: false,
  saved: true,
  ...over,
});

const realReplace = useDraft.getState().replaceText;
let replaceText: ReturnType<typeof vi.fn<(text: string) => Promise<boolean>>>;

function seed(rec: PlanRecord, status: "done" | "running" = "done", boot: string | null = "b") {
  usePlan.setState({
    status,
    record: rec,
    report: status === "done" ? "# Sizing plan s4\n" : null,
    error: null,
    sent: { target: { multiple: 2 }, boot },
  });
}

async function open() {
  act(() => usePlan.getState().setOpen(true));
  render(<PlanHost />);
  // the first open fetches the lazy chunk, slow under a full parallel run
  return screen.findByRole("dialog", { name: "Sizing plan" }, { timeout: 5000 });
}

const start = () => screen.getByRole("button", { name: "Start" });

// fetch the lazy chunk once up front: a slow first fetch under a full parallel run raced the first test
beforeAll(() => import("./PlanDialog"));

beforeEach(() => {
  sessionStorage.setItem(TOKEN_KEY, "tok");
  useMode.setState({ mode: "batch" });
  useRun.setState({ status: "idle", latest: null, dimmed: false, lastError: null });
  replaceText = vi.fn(async (_text: string) => true);
  useDraft.setState({ view: view(), replaceText });
  useToasts.setState({ toasts: [] });
  usePlan.getState().reset();
  usePlan.getState().setForm(DEFAULT_FORM);
});
afterEach(() => {
  cleanup();
  usePlan.getState().reset();
  usePlan.getState().setOpen(false);
  useDraft.setState({ replaceText: realReplace });
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  sessionStorage.clear();
});

test("Start posts the body built from the form", async () => {
  const sent = mockFetch([{ status: 202, json: { id: "s4" } }, { json: planRunning() }]);
  await open();
  fireEvent.change(screen.getByLabelText("Target"), { target: { value: "3" } });
  fireEvent.change(screen.getByLabelText(/p99 limit/), { target: { value: "500" } });
  fireEvent.change(screen.getByLabelText(/Budget/), { target: { value: "2.5" } });
  fireEvent.click(screen.getByLabelText("cache"));
  expect(usePlan.getState().form).toMatchObject({ target: "3", p99: "500", keep: ["cache"] });
  expect(screen.getByLabelText("Target")).toHaveProperty("value", "3");
  fireEvent.click(start());
  await vi.waitFor(() => expect(sent.length).toBeGreaterThan(0));
  expect(sent[0]?.url).toMatch(/\/sim\/plan$/);
  expect(sent[0]?.body).toEqual({
    target: { multiple: 3 },
    p99_ms: 500,
    error_pct: 1,
    budget_per_hour: 2.5,
    keep: ["cache"],
  });
  expect(await screen.findByText(/^Planning: 1 step so far/)).toBeTruthy();
  expect(screen.getByRole("button", { name: "Planning..." })).toHaveProperty("disabled", true);
});

test("an invalid field blocks Start and is described", async () => {
  const sent = mockFetch([{ status: 202, json: { id: "s4" } }]);
  await open();
  const target = screen.getByLabelText("Target");
  fireEvent.change(target, { target: { value: "0" } });
  expect(target.getAttribute("aria-invalid")).toBe("true");
  const why = document.getElementById(target.getAttribute("aria-describedby") ?? "");
  expect(why?.textContent).toBe("The target must be 0.1 to 64 times the profile's rates.");
  expect(start()).toHaveProperty("disabled", true);
  expect(start().getAttribute("aria-describedby")).toContain(why?.id);
  fireEvent.click(start());
  expect(sent).toHaveLength(0);
});

test("in Live and Admin Start works and the dialog says what a plan is of", async () => {
  const note =
    "Plans the applied design, not the live numbers; a change not applied yet is not in it.";
  for (const mode of ["live", "admin"] as const) {
    useMode.setState({ mode });
    useDraft.setState({ view: view({ dirty: true }) });
    const { unmount } = render(<PlanHost />);
    act(() => usePlan.getState().setOpen(true));
    await screen.findByRole("dialog", { name: "Sizing plan" }, { timeout: 5000 });
    expect(start(), mode).toHaveProperty("disabled", false);
    expect(screen.getByText(note), mode).toBeTruthy();
    expect(screen.queryByText(/Apply to include your edits/)).toBeNull();
    unmount();
    act(() => usePlan.getState().setOpen(false));
  }
});

test("Start from Live posts the plan", async () => {
  const sent = mockFetch([{ status: 202, json: { id: "s4" } }, { json: planRunning() }]);
  useMode.setState({ mode: "live" });
  await open();
  fireEvent.click(start());
  await vi.waitFor(() => expect(sent.length).toBeGreaterThan(0));
  expect(sent[0]?.url).toMatch(/\/sim\/plan$/);
});

test("a design with no traffic profile disables Start with a note", async () => {
  useDraft.setState({ view: view({ doc: { nodes: {}, edges: [] } }) });
  await open();
  expect(start()).toHaveProperty("disabled", true);
  expect(screen.getByText("The design has no traffic profile to plan from.")).toBeTruthy();
});

test("a dirty draft says the applied design is planned", async () => {
  useDraft.setState({ view: view({ dirty: true }) });
  await open();
  expect(screen.getByText("Plans the applied design; Apply to include your edits.")).toBeTruthy();
  expect(start()).toHaveProperty("disabled", false);
});

test("a busy answer shows the server's reason as a plain note, not an alert", async () => {
  mockFetch([{ status: 409, json: { error: "a run is in progress" } }]);
  await open();
  fireEvent.click(start());
  expect(await screen.findByText("a run is in progress")).toBeTruthy();
  expect(screen.queryByRole("alert")).toBeNull();
});

test("a 409 that waiting will not cure shows its own reason", async () => {
  const why =
    "the design in force differs from the applied text; Apply it (or retry if an Apply just ran)";
  mockFetch([{ status: 409, json: { error: why } }]);
  await open();
  fireEvent.click(start());
  expect(await screen.findByText(why)).toBeTruthy();
});

test("closing and reopening keeps a running plan's progress", async () => {
  seed(planRunning(), "running");
  await open();
  expect(screen.getByText(/^Planning: 1 step so far/)).toBeTruthy();
  fireEvent.change(screen.getByLabelText("Target"), { target: { value: "4" } });
  act(() => usePlan.getState().setOpen(false));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(usePlan.getState().status).toBe("running");
  act(() => usePlan.getState().setOpen(true));
  await screen.findByRole("dialog", { name: "Sizing plan" });
  expect(screen.getByText(/^Planning: 1 step so far/)).toBeTruthy();
  expect(screen.getByLabelText("Target")).toHaveProperty("value", "4");
});

test("Escape closes the dialog through the store", async () => {
  await open();
  fireEvent(screen.getByRole("dialog"), new Event("cancel", { cancelable: true }));
  expect(usePlan.getState().open).toBe(false);
});

test("Load on a clean draft calls replaceText with proposed_draft and closes", async () => {
  seed(planDone());
  await open();
  fireEvent.click(screen.getByRole("button", { name: "Load proposed draft" }));
  await vi.waitFor(() => expect(usePlan.getState().open).toBe(false));
  expect(replaceText).toHaveBeenCalledWith("version: 1\n");
  expect(useToasts.getState().toasts.map((t) => [t.kind, t.text])).toContainEqual([
    "success",
    "Loaded the proposed draft. Apply it when you are ready.",
  ]);
});

test("a refused load keeps the dialog open and says so", async () => {
  replaceText.mockResolvedValue(false);
  seed(planDone());
  await open();
  fireEvent.click(screen.getByRole("button", { name: "Load proposed draft" }));
  await vi.waitFor(() => expect(useToasts.getState().toasts).toHaveLength(1));
  expect(useToasts.getState().toasts[0]?.kind).toBe("error");
  expect(usePlan.getState().open).toBe(true);
});

test("Load on a dirty draft shows the confirm and does not call replaceText until Replace", async () => {
  useDraft.setState({ view: view({ dirty: true }) });
  seed(planDone());
  await open();
  fireEvent.click(screen.getByRole("button", { name: "Load proposed draft" }));
  expect(
    await screen.findByText(
      "This replaces your unsaved edits. They cannot be brought back with Undo.",
    ),
  ).toBeTruthy();
  expect(screen.queryByText("This plan was made on an earlier design.")).toBeNull();
  expect(replaceText).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Replace" }));
  await vi.waitFor(() => expect(replaceText).toHaveBeenCalledWith("version: 1\n"));
  await vi.waitFor(() => expect(usePlan.getState().open).toBe(false));
});

test("Cancel leaves the draft alone and focus on Load", async () => {
  useDraft.setState({ view: view({ dirty: true }) });
  seed(planDone());
  await open();
  const load = screen.getByRole("button", { name: "Load proposed draft" });
  fireEvent.click(load);
  const cancel = await screen.findByRole("button", { name: "Cancel" });
  expect(document.activeElement).toBe(cancel);
  fireEvent.click(cancel);
  expect(screen.queryByText(/This replaces your unsaved edits/)).toBeNull();
  expect(document.activeElement).toBe(screen.getByRole("button", { name: "Load proposed draft" }));
  expect(replaceText).not.toHaveBeenCalled();
  expect(usePlan.getState().open).toBe(true);
});

test("Copy my draft writes view.text", async () => {
  const writeText = vi.fn(() => Promise.resolve());
  vi.stubGlobal("navigator", { clipboard: { writeText } });
  useDraft.setState({ view: view({ dirty: true, text: "my edits\n" }) });
  seed(planDone());
  await open();
  fireEvent.click(screen.getByRole("button", { name: "Load proposed draft" }));
  fireEvent.click(await screen.findByRole("button", { name: "Copy my draft" }));
  expect(writeText).toHaveBeenCalledWith("my edits\n");
  expect(await screen.findByText("Copied")).toBeTruthy();
  expect(replaceText).not.toHaveBeenCalled();
});

test("a plan with no proposed_draft has no Load button", async () => {
  seed(planCannotHold());
  await open();
  expect(screen.getByText(/The plan cannot hold the target by sizing at 'db'/)).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Load proposed draft" })).toBeNull();
  cleanup();
  seed({ ...planDone(), proposed_draft: "" });
  await open();
  expect(screen.queryByRole("button", { name: "Load proposed draft" })).toBeNull();
});

test("a plan from an earlier generation says so", async () => {
  useDraft.setState({ view: view({ generation: 3, dirty: true }) });
  seed(planDone());
  await open();
  expect(screen.getByText(/^Made on the applied design \(generation 2\) for /)).toBeTruthy();
  expect(
    screen.getByText(
      "Made on an earlier design: the design was applied again since. Start a new plan to see the design in force.",
    ),
  ).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Load proposed draft" }));
  expect(await screen.findByText("This plan was made on an earlier design.")).toBeTruthy();
});

test("a plan from before a server restart says so", async () => {
  seed(planDone(), "done", "old");
  await open();
  expect(
    screen.getByText(
      "Made before the server restarted. Start a new plan to see the design in force.",
    ),
  ).toBeTruthy();
});

test("the cheapest fields are sent only when the box is checked", async () => {
  const sent = mockFetch([{ status: 409, json: { error: "busy" } }]);
  await open();
  // each start is answered busy: wait for that before the next click
  const settled = (n: number) =>
    vi.waitFor(() => {
      expect(sent).toHaveLength(n);
      expect(usePlan.getState().status).toBe("busy");
    });
  expect(screen.queryByLabelText(/Time limit/)).toBeNull();
  fireEvent.click(start());
  await settled(1);
  expect(sent[0]?.body).toEqual({ target: { multiple: 2 }, p99_ms: 1000, error_pct: 1 });
  fireEvent.click(screen.getByLabelText("Also look for the cheapest design"));
  fireEvent.click(screen.getByLabelText("GCP"));
  fireEvent.click(screen.getByLabelText("Azure"));
  fireEvent.click(start());
  await settled(2);
  expect(sent[1]?.body).toMatchObject({
    cheapest: true,
    time_limit_s: 150,
    run_cap: 200,
    providers: ["gcp", "azure"],
  });
  // "none" clears the providers, and a provider clears "none"
  fireEvent.click(screen.getByLabelText("None"));
  expect(screen.getByLabelText("GCP")).toHaveProperty("checked", false);
  fireEvent.click(start());
  await settled(3);
  expect(sent[2]?.body).toMatchObject({ providers: ["none"] });
  fireEvent.click(screen.getByLabelText("AWS"));
  expect(screen.getByLabelText("None")).toHaveProperty("checked", false);
  fireEvent.click(screen.getByLabelText("Also look for the cheapest design"));
  fireEvent.click(start());
  await settled(4);
  expect(sent[3]?.body).toEqual({ target: { multiple: 2 }, p99_ms: 1000, error_pct: 1 });
});

test("a finished cheapest plan shows its parts and the report buttons", async () => {
  seed(planCheapest());
  await open();
  expect(screen.getAllByText("The plan holds the target.").length).toBeGreaterThan(0);
  expect(screen.getByText(/the plan shown costs \$0.94\/h/)).toBeTruthy();
  expect(screen.getByRole("table", { name: "Changes" })).toBeTruthy();
  expect(screen.getByRole("table", { name: "Comparison" })).toBeTruthy();
  expect(screen.getByText("cheapest found within the limit, not proven cheapest")).toBeTruthy();
  expect(screen.getByText(/this search's share of the 150 s time limit/)).toBeTruthy();
  expect(
    screen.getByText(/finds its knee near 262 req\/s \(2.62x the profile's rates\)/),
  ).toBeTruthy();
  expect(screen.getByText(/It holds the target with the margin/)).toBeTruthy();
  expect(screen.getByRole("button", { name: "Copy as Markdown" })).toBeTruthy();
});

test("an older server's record renders with a plain verdict line", async () => {
  const rec = planDone();
  delete rec.plan!.words;
  rec.plan!.measured_knee = null;
  rec.plan!.sweep_error = "no profile";
  seed(rec);
  await open();
  expect(screen.getByText(/^Verdict: holds \(this server sent no sentence/)).toBeTruthy();
  expect(
    screen.getByText("The load sweep of the proposed design did not run: no profile."),
  ).toBeTruthy();
  expect(screen.queryByRole("table", { name: "Comparison" })).toBeNull();
});

test("the dialog has no axe violations", async () => {
  useDraft.setState({ view: view({ dirty: true, generation: 3 }) });
  seed(planCheapest());
  const dialog = await open();
  fireEvent.click(screen.getByLabelText("Also look for the cheapest design"));
  fireEvent.click(screen.getByRole("button", { name: "Load proposed draft" }));
  await expectNoViolations(dialog, { page: false });
});

test("a ticked node that is no longer in the draft is not sent", async () => {
  const sent = mockFetch([{ status: 202, json: { id: "s4" } }, { json: planRunning() }]);
  usePlan.getState().setForm({ ...DEFAULT_FORM, keep: ["cache", "gone"] });
  await open();
  expect(screen.getByText("Keep as they are (1 of 3)")).toBeTruthy();
  fireEvent.click(start());
  await vi.waitFor(() => expect(sent.length).toBeGreaterThan(0));
  expect(sent[0]?.body).toMatchObject({ keep: ["cache"] });
});

test("the live region holds the step count only; the clock is outside it", async () => {
  seed(planRunning(), "running");
  await open();
  const live = await screen.findByText(/^Planning: 1 step so far/);
  expect(live.getAttribute("aria-live")).toBe("polite");
  expect(live.textContent).not.toMatch(/\d+s so far|ms so far/);
  expect(screen.getByText(/so far$/, { selector: "p" })).toBeTruthy();
});

test("a done plan with no plan in it says so", async () => {
  seed({ ...planDone(), plan: null });
  await open();
  expect(await screen.findByText("No plan came back from the server.")).toBeTruthy();
});

test("the foot says what the plan was asked for", async () => {
  seed(planDone());
  await open();
  expect(
    await screen.findByText(
      "Made on the applied design (generation 2) for 2.5x the profile's rates (250 req/s), p99 1000 ms, errors 1%",
    ),
  ).toBeTruthy();
});

test("Replace pressed twice loads once", async () => {
  let release: (ok: boolean) => void = () => {};
  replaceText.mockImplementation(() => new Promise<boolean>((r) => (release = r)));
  useDraft.setState({ view: view({ dirty: true }) });
  seed(planDone());
  await open();
  fireEvent.click(await screen.findByRole("button", { name: "Load proposed draft" }));
  const replace = await screen.findByRole("button", { name: "Replace" });
  fireEvent.click(replace);
  fireEvent.click(replace);
  expect(replaceText).toHaveBeenCalledTimes(1);
  await act(async () => release(true));
});

test("a refused load also says so inside the dialog, as an alert", async () => {
  replaceText.mockResolvedValue(false);
  seed(planDone());
  await open();
  fireEvent.click(await screen.findByRole("button", { name: "Load proposed draft" }));
  expect((await screen.findByRole("alert")).textContent).toMatch(
    /Could not load the proposed draft/,
  );
});

test("Copy my draft with no draft in the store writes nothing", async () => {
  const writeText = vi.fn(async () => {});
  vi.stubGlobal("navigator", { clipboard: { writeText } });
  useDraft.setState({ view: view({ dirty: true }) });
  seed(planDone());
  await open();
  fireEvent.click(await screen.findByRole("button", { name: "Load proposed draft" }));
  const copy = await screen.findByRole("button", { name: "Copy my draft" });
  act(() => useDraft.setState({ view: null }));
  fireEvent.click(copy);
  expect(writeText).not.toHaveBeenCalled();
  expect(screen.getByText("Nothing to copy yet")).toBeTruthy();
});

test("a stale plan is still offered with the stale confirm for a dirty draft", async () => {
  useDraft.setState({ view: view({ dirty: true, generation: 5 }) });
  seed(planDone());
  await open();
  fireEvent.click(await screen.findByRole("button", { name: "Load proposed draft" }));
  expect(await screen.findByText("This plan was made on an earlier design.")).toBeTruthy();
  expect(replaceText).not.toHaveBeenCalled();
});

test("a running record with no plan_steps yet (the server leaves them out) renders", async () => {
  const { plan_steps: _gone, ...bare } = planRunning();
  seed(bare as PlanRecord, "running");
  await open();
  expect(await screen.findByText("Planning: 0 steps so far")).toBeTruthy();
});

test("a stale plan asks before it loads into a clean draft", async () => {
  useDraft.setState({ view: view({ generation: 5 }) });
  seed(planDone());
  await open();
  fireEvent.click(await screen.findByRole("button", { name: "Load proposed draft" }));
  expect(await screen.findByText("This plan was made on an earlier design.")).toBeTruthy();
  expect(screen.getByText(/changes applied since are not in it/)).toBeTruthy();
  expect(replaceText).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Replace" }));
  await vi.waitFor(() => expect(replaceText).toHaveBeenCalledTimes(1));
});

test("a dirty draft's keep list says its names are checked against the applied design", async () => {
  useDraft.setState({ view: view({ dirty: true }) });
  await open();
  fireEvent.click(screen.getByText(/^Keep as they are/));
  expect(screen.getByText(/the plan checks them against the applied design/)).toBeTruthy();
});

test("two edits in one batch both land, because each starts from the latest form", async () => {
  await open();
  act(() => {
    fireEvent.change(screen.getByLabelText("Target"), { target: { value: "4" } });
    fireEvent.change(screen.getByLabelText(/p99 limit/), { target: { value: "750" } });
    fireEvent.click(screen.getByLabelText("cache"));
  });
  expect(usePlan.getState().form).toMatchObject({ target: "4", p99: "750", keep: ["cache"] });
});
