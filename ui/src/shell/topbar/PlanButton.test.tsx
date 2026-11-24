import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test } from "vite-plus/test";
import type { DraftView } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { useMode } from "../../data/store/mode";
import { usePlan } from "../../data/store/plan";
import { expectNoViolations } from "../../test/axe";
import { planDone, planRunning } from "../../test/planRecord";
import { PlanButton, PlanMark } from "./PlanButton";

const view = (generation = 2): DraftView => ({
  text: "",
  dirty: false,
  generation,
  lost: [],
  rev: 0,
  boot: "b",
  doc: { nodes: {}, edges: [], traffic: { peak: {} } },
  doc_error: null,
  project_id: "p",
  project_name: null,
  kind_defaults: {},
  layout: {},
  layout_note: null,
  layout_dirty: false,
  saved: true,
});

beforeEach(() => {
  useMode.setState({ mode: "batch" });
  useDraft.setState({ view: view() });
  usePlan.getState().reset();
  usePlan.getState().setOpen(false);
});
afterEach(() => {
  cleanup();
  usePlan.getState().reset();
  usePlan.getState().setOpen(false);
});

test("the button opens the dialog through the store", () => {
  render(<PlanButton />);
  fireEvent.click(screen.getByRole("button", { name: "Plan" }));
  expect(usePlan.getState().open).toBe(true);
});

test("it opens the dialog in every mode", () => {
  for (const mode of ["batch", "live", "admin"] as const) {
    useMode.setState({ mode });
    usePlan.getState().setOpen(false);
    const { unmount } = render(<PlanButton />);
    const b = screen.getByRole("button", { name: "Plan" });
    expect(b).toHaveProperty("disabled", false);
    fireEvent.click(b);
    expect(usePlan.getState().open, mode).toBe(true);
    unmount();
  }
});

test("the marker says running, done, earlier design and failed in words", () => {
  const { rerender } = render(
    <>
      <PlanButton />
      <PlanMark />
    </>,
  );
  expect(screen.queryByRole("status")).toBeNull();
  usePlan.setState({
    status: "running",
    record: planRunning(),
    sent: { target: { multiple: 2 }, boot: "b" },
  });
  rerender(
    <>
      <PlanButton />
      <PlanMark />
    </>,
  );
  expect(screen.getByText("Plan: running")).toBeTruthy();
  usePlan.setState({ status: "done", record: planDone() });
  rerender(
    <>
      <PlanButton />
      <PlanMark />
    </>,
  );
  expect(screen.getByText("Plan: done")).toBeTruthy();
  useDraft.setState({ view: view(5) });
  rerender(
    <>
      <PlanButton />
      <PlanMark />
    </>,
  );
  expect(screen.getByText("Plan: earlier design")).toBeTruthy();
  usePlan.setState({ status: "failed" });
  rerender(
    <>
      <PlanButton />
      <PlanMark />
    </>,
  );
  expect(screen.getByText("Plan: failed")).toBeTruthy();
});

test("a plan from before a server restart reads as an earlier design", () => {
  usePlan.setState({
    status: "done",
    record: planDone(),
    sent: { target: { multiple: 2 }, boot: "old" },
  });
  render(
    <>
      <PlanButton />
      <PlanMark />
    </>,
  );
  expect(screen.getByText("Plan: earlier design")).toBeTruthy();
});

test("the button has no axe violations", async () => {
  const { container } = render(<PlanButton />);
  await expectNoViolations(container);
});
