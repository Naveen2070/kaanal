import { cleanup, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { useRouteOwners } from "../../data/store/owners";
import { nodeSummary } from "../../test/summary";
import { app, show } from "./controlsHarness";
import { useGroups } from "./groups/store";

beforeEach(() => {
  localStorage.clear();
  useGroups.setState({ open: {} });
});
afterEach(() => {
  cleanup();
  vi.mocked(useRouteOwners).mockReturnValue({});
});

test("the_steps_group_sits_on_the_component_the_server_names_as_the_owner", async () => {
  vi.mocked(useRouteOwners).mockReturnValue({ "GET /users": "app", "GET /pay": "other" });
  await show(nodeSummary(), { ...app });
  expect(await screen.findByRole("combobox", { name: "Route" }, { timeout: 8000 })).toBeTruthy();
  const routes = [
    ...(screen.getByRole("combobox", { name: "Route" }) as HTMLSelectElement).options,
  ];
  expect(routes.map((o) => o.value)).toEqual(["GET /users"]);
});

test("a_component_that_owns_no_route_has_no_steps_group", async () => {
  vi.mocked(useRouteOwners).mockReturnValue({ "GET /pay": "other" });
  await show(nodeSummary(), { ...app });
  expect(screen.queryByRole("combobox", { name: "Route" })).toBeNull();
});
