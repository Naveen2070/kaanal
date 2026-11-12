import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vite-plus/test";
import * as payload from "../../data/api/payload";
import { useMode } from "../../data/store/mode";
import { EditResponse } from "./EditResponse";

afterEach(() => {
  cleanup();
  useMode.setState({ mode: "live" });
  vi.restoreAllMocks();
});

test("edit_response_is_offered_for_a_route_in_admin_only_and_says_when_it_is_edited", () => {
  vi.spyOn(payload, "listPayloads").mockResolvedValue([]);
  useMode.setState({ mode: "live" });
  const r = render(<EditResponse route="GET /x" />);
  expect(screen.queryByRole("button")).toBeNull();
  useMode.setState({ mode: "admin" });
  r.rerender(<EditResponse route="GET /x" />);
  expect(screen.getByRole("button", { name: "Edit response of GET /x" }).textContent).toBe("Edit");
  r.rerender(<EditResponse route="GET /x" edited />);
  expect(screen.getByRole("button", { name: "Edit response of GET /x" }).textContent).toMatch(
    /edited/,
  );
});
