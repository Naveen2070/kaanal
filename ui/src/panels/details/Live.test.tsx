import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vite-plus/test";
import type { PinsView, ScenarioRow } from "../../data/api/types";
import { Live } from "./Live";

interface Reply {
  status?: number;
  json?: unknown;
}
interface Seen {
  key: string;
  body: unknown;
}

/** A fetch that answers by `METHOD /path`; the requests are kept to look at. */
function serve(routes: Record<string, Reply>): Seen[] {
  const seen: Seen[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string, init: RequestInit = {}) => {
      const key = `${init.method ?? "GET"} ${String(url).replace("/__kaanal", "")}`;
      seen.push({ key, body: typeof init.body === "string" ? JSON.parse(init.body) : undefined });
      const r = routes[key] ?? { status: 204 };
      return new Response(r.json === undefined ? "" : JSON.stringify(r.json), {
        status: r.status ?? 200,
      });
    }),
  );
  return seen;
}

const scenarios: ScenarioRow[] = [
  {
    name: "db-down",
    source: "file",
    description: "The database is down",
    entries: 2,
    active: false,
  },
  { name: "slow-pay", source: "inline", entries: 1, active: true, expires_in_ms: 29_100 },
];
const pins: PinsView = {
  live: [{ endpoint: "GET /orders", state: "error", status: 429, delay_ms: 500 }],
  effective: {
    "GET /orders": {
      endpoint: "GET /orders",
      state: "error",
      status: 429,
      delay_ms: 500,
      source: "live",
    },
    "GET /users": { endpoint: "GET /users", state: "empty", source: { scenario: "slow-pay" } },
  },
};
const system: Record<string, Reply> = {
  "GET /scenarios": { json: scenarios },
  "GET /pins": { json: pins },
  "GET /topology": {
    json: {
      nodes: [],
      edges: [],
      routes: [],
      endpoints: [
        { key: "GET /orders", visits: {} },
        { key: "POST /orders", visits: {} },
      ],
    },
  },
};

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

test("with_no_system_applied_it_says_what_live_is_and_offers_apply_and_start", async () => {
  serve({ "GET /scenarios": { status: 409, json: { error: "no system applied yet" } } });
  render(<Live />);
  expect(await screen.findByText(/There is no running system yet/)).toBeTruthy();
  expect(screen.queryByRole("button", { name: /Turn on/ })).toBeNull();
  expect(screen.getByRole("button", { name: "Apply and start" })).toBeTruthy();
});

test("apply_and_start_presses_the_top_bars_apply", async () => {
  serve({ "GET /scenarios": { status: 409, json: { error: "no system applied yet" } } });
  const pressed = vi.fn();
  render(
    <>
      <button type="button" data-command="apply" onClick={pressed}>
        Apply
      </button>
      <Live />
    </>,
  );
  fireEvent.click(await screen.findByRole("button", { name: "Apply and start" }));
  expect(pressed).toHaveBeenCalledTimes(1);
  // while it waits, a second click cannot press Apply again
  expect(
    (screen.getByRole("button", { name: "Apply and start" }) as HTMLButtonElement).disabled,
  ).toBe(true);
});

test("scenarios_and_pins_are_listed_with_where_they_come_from", async () => {
  serve(system);
  render(<Live />);
  expect(await screen.findByText("db-down")).toBeTruthy();
  expect(screen.getByText(/The database is down. 2 entries, in scenarios\//)).toBeTruthy();
  expect(screen.getByText(/1 entry, in kaanal.yaml, 30 s left/)).toBeTruthy();
  expect(
    screen.getByRole("button", { name: "Turn off slow-pay" }).getAttribute("aria-pressed"),
  ).toBe("true");
  expect(screen.getByText("error 429, after 500 ms")).toBeTruthy();
  // a pin that comes from a scenario is shown, and cannot be stepped or removed from here
  expect(screen.getByText(/empty, from the scenario slow-pay/)).toBeTruthy();
  expect(screen.getAllByRole("button", { name: /Next state for/ })).toHaveLength(1);
});

test("turning_a_scenario_on_sends_the_lease_and_off_sends_a_deactivate", async () => {
  const seen = serve(system);
  render(<Live />);
  await screen.findByText("db-down");
  fireEvent.change(screen.getByLabelText("Turn on"), { target: { value: "1" } });
  fireEvent.click(screen.getByRole("button", { name: "Turn on db-down" }));
  await vi.waitFor(() =>
    expect(seen.map((s) => s.key)).toContain("POST /scenarios/db-down/activate"),
  );
  expect(seen.find((s) => s.key.endsWith("db-down/activate"))?.body).toEqual({ ttl: "30s" });

  fireEvent.click(screen.getByRole("button", { name: "Turn off slow-pay" }));
  await vi.waitFor(() =>
    expect(seen.map((s) => s.key)).toContain("POST /scenarios/slow-pay/deactivate"),
  );
  fireEvent.click(screen.getByRole("button", { name: "Turn all off" }));
  await vi.waitFor(() => expect(seen.map((s) => s.key)).toContain("DELETE /scenarios"));
});

test("the_save_box_is_there_with_and_without_scenarios", async () => {
  serve(system);
  render(<Live />);
  await screen.findByText("db-down");
  expect(screen.getByRole("button", { name: "Save live" })).toBeTruthy();
  cleanup();
  serve({ ...system, "GET /scenarios": { json: [] } });
  render(<Live />);
  expect(await screen.findByText(/None are defined/)).toBeTruthy();
  expect(screen.getByRole("button", { name: "Save live" })).toBeTruthy();
});

test("a_pin_the_api_would_refuse_is_explained_and_never_sent", async () => {
  const seen = serve(system);
  render(<Live />);
  await screen.findByText("db-down");
  fireEvent.click(screen.getByRole("button", { name: "Pin it" }));
  expect((await screen.findByRole("alert")).textContent).toBe("Pick an endpoint.");
  fireEvent.change(screen.getByLabelText("Pin"), { target: { value: "POST /orders" } });
  fireEvent.change(screen.getByLabelText("status"), { target: { value: "200" } });
  fireEvent.click(screen.getByRole("button", { name: "Pin it" }));
  await vi.waitFor(() => expect(screen.getByRole("alert").textContent).toMatch(/400 to 599/));
  expect(seen.some((s) => s.key.startsWith("PUT"))).toBe(false);
});

test("a_valid_pin_is_sent_and_a_cycle_and_a_removal_name_their_endpoint", async () => {
  const seen = serve({ ...system, "PUT /pins": { json: {} }, "POST /cycle": { json: {} } });
  render(<Live />);
  await screen.findByText("db-down");
  fireEvent.change(screen.getByLabelText("Pin"), { target: { value: "POST /orders" } });
  fireEvent.change(screen.getByLabelText("status"), { target: { value: "503" } });
  fireEvent.change(screen.getByLabelText("delay"), { target: { value: "250" } });
  fireEvent.click(screen.getByRole("button", { name: "Pin it" }));
  await vi.waitFor(() => expect(seen.map((s) => s.key)).toContain("PUT /pins"));
  expect(seen.find((s) => s.key === "PUT /pins")?.body).toEqual({
    endpoint: "POST /orders",
    state: "error",
    status: 503,
    delay: "250ms",
  });

  fireEvent.click(screen.getByRole("button", { name: "Next state for GET /orders" }));
  await vi.waitFor(() => expect(seen.map((s) => s.key)).toContain("POST /cycle"));
  expect(seen.find((s) => s.key === "POST /cycle")?.body).toEqual({ endpoint: "GET /orders" });

  fireEvent.click(screen.getByRole("button", { name: "Remove the pin on GET /orders" }));
  await vi.waitFor(() =>
    expect(seen.map((s) => s.key)).toContain("DELETE /pins?endpoint=GET+%2Forders"),
  );
});

test("the_state_decides_which_boxes_show", async () => {
  serve(system);
  render(<Live />);
  await screen.findByText("db-down");
  expect(screen.getByLabelText("status")).toBeTruthy(); // error: a status and a delay
  fireEvent.change(screen.getByLabelText("as"), { target: { value: "slow" } });
  expect(screen.queryByLabelText("status")).toBeNull();
  expect(screen.getByLabelText("delay")).toBeTruthy();
  fireEvent.change(screen.getByLabelText("as"), { target: { value: "success" } });
  expect(screen.queryByLabelText("delay")).toBeNull();
});

test("a_sample_shows_status_size_and_body_and_sends_the_seed", async () => {
  const seen = serve({
    ...system,
    "POST /sample": {
      json: {
        endpoint: "GET /orders",
        status: 200,
        body: { id: 7, items: [] },
        bytes: 1536,
        label: "relative what-if, not a prediction",
      },
    },
  });
  render(<Live />);
  await screen.findByText("db-down");
  const show = screen.getByRole("button", { name: "Show a sample" }) as HTMLButtonElement;
  expect(show.disabled).toBe(true); // no endpoint yet
  fireEvent.change(screen.getByLabelText("Endpoint"), { target: { value: "GET /orders" } });
  fireEvent.change(screen.getByLabelText("Seed"), { target: { value: "7" } });
  fireEvent.click(show);
  expect(await screen.findByText(/GET \/orders answers 200, 1,536 bytes/)).toBeTruthy();
  expect(screen.getByLabelText("Sample body").textContent).toContain('"id": 7');
  expect(seen.find((s) => s.key === "POST /sample")?.body).toEqual({
    endpoint: "GET /orders",
    seed: 7,
  });
});

test("a_route_with_no_schema_says_so_and_a_refusal_is_shown", async () => {
  serve({
    ...system,
    "POST /sample": {
      json: { endpoint: "GET /orders", status: 200, body: null, bytes: 0, label: "x" },
    },
  });
  render(<Live />);
  await screen.findByText("db-down");
  fireEvent.change(screen.getByLabelText("Endpoint"), { target: { value: "GET /orders" } });
  fireEvent.click(screen.getByRole("button", { name: "Show a sample" }));
  expect(await screen.findByText(/no response schema/)).toBeTruthy();
  cleanup();
  vi.unstubAllGlobals();

  serve({ ...system, "POST /sample": { status: 404, json: { error: "no route 'GET /orders'" } } });
  render(<Live />);
  await screen.findByText("db-down");
  fireEvent.change(screen.getByLabelText("Endpoint"), { target: { value: "GET /orders" } });
  fireEvent.click(screen.getByRole("button", { name: "Show a sample" }));
  expect((await screen.findByRole("alert")).textContent).toBe("no route 'GET /orders'");
  // a bad seed is caught before asking
  fireEvent.change(screen.getByLabelText("Seed"), { target: { value: "-3" } });
  fireEvent.click(screen.getByRole("button", { name: "Show a sample" }));
  await vi.waitFor(() =>
    expect(screen.getByRole("alert").textContent).toBe("The seed is a whole number, 0 or more."),
  );
});
