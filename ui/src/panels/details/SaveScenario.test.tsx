import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { useToasts } from "../../shell/toast/toasts";
import { SaveScenario } from "./SaveScenario";

interface Reply {
  status?: number;
  json?: unknown;
}

/** A fetch that answers the save route from a queue; the bodies sent are kept. */
function serve(...replies: Reply[]): unknown[] {
  const bodies: unknown[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (_url: string, init: RequestInit = {}) => {
      bodies.push(JSON.parse(init.body as string));
      const r = replies.shift() ?? { json: { name: "x", path: "p", warnings: [] } };
      return new Response(JSON.stringify(r.json), { status: r.status ?? 200 });
    }),
  );
  return bodies;
}

const ok = (name: string, warnings: string[] = []): Reply => ({
  json: { name, path: `/p/scenarios/${name}.yaml`, warnings },
});
const name = () => screen.getByLabelText("Scenario name") as HTMLInputElement;
const save = () => screen.getByRole("button", { name: "Save live" }) as HTMLButtonElement;
const type = (v: string) => fireEvent.change(name(), { target: { value: v } });

beforeEach(() => useToasts.setState({ toasts: [] }));
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

test("saving_sends_the_live_layer_clears_the_box_and_tells_the_list", async () => {
  const bodies = serve(ok("mine"));
  const onSaved = vi.fn();
  render(<SaveScenario onSaved={onSaved} />);
  expect(save().disabled).toBe(true);
  type(" mine ");
  fireEvent.click(save());
  await vi.waitFor(() => expect(onSaved).toHaveBeenCalledTimes(1));
  expect(bodies).toEqual([{ name: "mine", from: "live" }]);
  expect(name().value).toBe("");
  expect(
    useToasts
      .getState()
      .toasts.map((t) => t.text)
      .join(),
  ).toMatch(/Saved.*mine/);
});

test("a_name_that_exists_asks_before_replacing_and_retries_with_force", async () => {
  const bodies = serve(
    { status: 409, json: { error: "scenario 'mine' already exists" } },
    ok("mine"),
  );
  const onSaved = vi.fn();
  render(<SaveScenario onSaved={onSaved} />);
  type("mine");
  fireEvent.click(save());
  expect(await screen.findByText("Replace scenarios/mine.yaml?")).toBeTruthy();
  expect(onSaved).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Replace" }));
  await vi.waitFor(() => expect(onSaved).toHaveBeenCalledTimes(1));
  expect(bodies[1]).toEqual({ name: "mine", from: "live", force: true });
});

test("editing_the_name_or_cancel_removes_the_replace_prompt", async () => {
  serve(
    { status: 409, json: { error: "scenario already exists" } },
    { status: 409, json: { error: "scenario already exists" } },
  );
  render(<SaveScenario onSaved={vi.fn()} />);
  type("mine");
  fireEvent.click(save());
  await screen.findByText("Replace scenarios/mine.yaml?");
  type("mine2");
  expect(screen.queryByText(/Replace scenarios/)).toBeNull();
  fireEvent.click(save());
  await screen.findByText("Replace scenarios/mine2.yaml?");
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(screen.queryByText(/Replace scenarios/)).toBeNull();
});

test("a_409_that_is_not_a_name_clash_is_an_error_not_a_replace_prompt", async () => {
  serve({ status: 409, json: { error: "the system changed while this ran; retry" } });
  render(<SaveScenario onSaved={vi.fn()} />);
  type("mine");
  fireEvent.click(save());
  expect((await screen.findByRole("alert")).textContent).toMatch(/system changed/);
  expect(screen.queryByRole("button", { name: "Replace" })).toBeNull();
});

test("a_refusal_shows_the_servers_words_and_no_toast", async () => {
  serve({ status: 400, json: { error: "nothing to save" } });
  const onSaved = vi.fn();
  render(<SaveScenario onSaved={onSaved} />);
  type("mine");
  fireEvent.click(save());
  expect((await screen.findByRole("alert")).textContent).toMatch(/nothing to save/);
  expect(onSaved).not.toHaveBeenCalled();
  expect(useToasts.getState().toasts).toHaveLength(0);
  expect(name().value).toBe("mine");
});

test("the_lease_warning_reaches_a_toast", async () => {
  serve(ok("mine", ["2 fields have a ttl; the file has none"]));
  render(<SaveScenario onSaved={vi.fn()} />);
  type("mine");
  fireEvent.click(save());
  await vi.waitFor(() =>
    expect(
      useToasts
        .getState()
        .toasts.some((t) => t.kind === "warn" && /2 fields have a ttl/.test(t.text)),
    ).toBe(true),
  );
});

test("a_second_click_while_saving_sends_nothing_more", async () => {
  const bodies = serve(ok("mine"));
  render(<SaveScenario onSaved={vi.fn()} />);
  type("mine");
  fireEvent.click(save());
  expect(save().disabled).toBe(true);
  fireEvent.click(save());
  await vi.waitFor(() => expect(name().value).toBe(""));
  expect(bodies).toHaveLength(1);
});
