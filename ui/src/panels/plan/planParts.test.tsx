import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, expect, test } from "vite-plus/test";
import type { PlanDoc } from "../../data/api/types.plan";
import { expectNoViolations } from "../../test/axe";
import { planCheapest, planDone } from "../../test/planRecord";
import { PlanComparison } from "./PlanComparison";
import { PlanFamilies } from "./PlanFamilies";
import { PlanSearch, span } from "./PlanSearch";
import { PlanSteps } from "./PlanSteps";

afterEach(cleanup);

const cheapest = (): PlanDoc => planCheapest().plan!;
const done = (): PlanDoc => planDone().plan!;
const HOSTILE = "<img src=x onerror=alert(1)>";
const LONG = "n".repeat(200);
const RLO = String.fromCharCode(0x202e); // right-to-left override

test("steps_render_one_row_per_step_with_exact_price_effects", () => {
  const doc = cheapest();
  render(<PlanSteps steps={doc.steps} />);
  const table = screen.getByRole("table");
  // header row + one row per step (no menus in this fixture)
  expect(within(table).getAllByRole("row")).toHaveLength(1 + doc.steps.length);
  expect(screen.getByText("-$0.40/h")).toBeTruthy();
  expect(screen.getByText("-$0.29/h")).toBeTruthy();
  expect(screen.getByText("-$0.0035/h")).toBeTruthy();
  expect(screen.getByText("copies 5 to 3")).toBeTruthy();
  expect(screen.getByText("cheaper by $0.40 an hour and still holds")).toBeTruthy();
});

test("a_menu_alternative_is_inside_details_and_says_whether_it_holds", () => {
  const { container } = render(<PlanSteps steps={done().steps} />);
  const details = container.querySelectorAll("details");
  expect(details).toHaveLength(1);
  const alt = within(details[0] as HTMLElement).getByText(
    /instance class db\.m5\.large to db\.m5\.xlarge/,
  );
  expect(alt.closest("details")).toBe(details[0]);
  expect(details[0]?.textContent).toMatch(/\$1\.13\/h/);
  expect(details[0]?.textContent).toMatch(/does not hold/);
  expect(details[0]?.open).toBe(false);
});

test("the_comparison_lists_not_compared_first_then_ranks_and_the_conclusion_verbatim", () => {
  const c = cheapest().comparison!;
  const { container } = render(<PlanComparison comparison={c} />);
  const skipped = screen.getByText(/no managed postgres class for azure/);
  const table = screen.getByRole("table");
  expect(skipped.compareDocumentPosition(table) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  const rows = within(table).getAllByRole("row").slice(1);
  expect(rows).toHaveLength(2);
  expect(within(rows[0] as HTMLElement).getAllByRole("cell")[0]?.textContent).toBe("1");
  expect(within(rows[1] as HTMLElement).getAllByRole("cell")[0]?.textContent).toBe("2");
  expect(screen.getByText("$0.892/h")).toBeTruthy();
  expect(container.textContent).toContain(c.conclusion);
});

test("a_row_with_no_rank_reads_not_comparable_only_when_it_holds", () => {
  const c = cheapest().comparison!;
  const rows = c.rows.map(({ rank: _rank, ...r }, i) =>
    i === 0 ? r : { ...r, holds: false, outcome: "does not hold" },
  );
  render(<PlanComparison comparison={{ ...c, rows, comparable: false }} />);
  const body = within(screen.getByRole("table")).getAllByRole("row").slice(1);
  expect(within(body[0] as HTMLElement).getAllByRole("cell")[0]?.textContent).toBe(
    "not comparable",
  );
  expect(within(body[1] as HTMLElement).getAllByRole("cell")[0]?.textContent).toBe("-");
  expect(screen.getByText("does not hold")).toBeTruthy();
});

test("hostile_and_long_names_render_as_text_and_keep_the_tables", () => {
  const doc = cheapest();
  const steps = doc.steps.map((s) => ({ ...s, mv: { ...s.mv, node: HOSTILE } }));
  steps[1] = { ...steps[1]!, mv: { ...steps[1]!.mv, node: LONG + RLO } };
  const c = doc.comparison!;
  const rows = c.rows.map((r) => ({ ...r, name: HOSTILE, unpriced_nodes: [LONG] }));
  const { container } = render(
    <>
      <PlanSteps steps={steps} />
      <PlanComparison comparison={{ ...c, rows }} />
    </>,
  );
  expect(container.querySelector("img")).toBeNull();
  expect(screen.getAllByText(HOSTILE).length).toBeGreaterThan(0);
  expect(container.textContent).not.toContain(RLO);
  expect(screen.getByText(LONG)).toBeTruthy(); // the step's node, override cleaned out
  const tables = screen.getAllByRole("table");
  expect(tables).toHaveLength(2);
  expect(within(tables[0] as HTMLElement).getAllByRole("row")).toHaveLength(4);
  expect(within(tables[1] as HTMLElement).getAllByRole("row")).toHaveLength(3);
});

test("an_older_servers_plan_renders_with_the_reason_not_a_crash", () => {
  const doc: PlanDoc = done();
  delete doc.comparison;
  delete doc.search;
  delete doc.providers;
  delete doc.words;
  delete doc.cheapest_note;
  const { container } = render(
    <>
      <PlanSteps steps={doc.steps} />
      <PlanComparison comparison={doc.comparison} />
      <PlanSearch search={doc.search} cheapestNote={doc.cheapest_note} />
    </>,
  );
  // header, two steps, and the first step's menu row
  expect(within(screen.getByRole("table")).getAllByRole("row")).toHaveLength(4);
  expect(screen.getByText("copies 2 to 3")).toBeTruthy();
  expect(screen.getByText("No comparison in this plan.")).toBeTruthy();
  expect(container.textContent).not.toMatch(/cheapest/i);
});

test("families_are_collapsed_details_with_their_notes_or_reason", () => {
  const { container } = render(<PlanFamilies families={done().families} />);
  const details = container.querySelectorAll("details");
  expect(details).toHaveLength(2);
  expect([...details].every((d) => !d.open)).toBe(true);
  expect(details[0]?.textContent).toMatch(/dynamodb/);
  expect(details[0]?.textContent).toMatch(/assumes your application can use this family/);
  expect(details[1]?.textContent).toMatch(/mongodb/);
  expect(details[1]?.textContent).toMatch(/no preset instance table for this flavor/);
});

test("no_families_draws_nothing", () => {
  const { container } = render(<PlanFamilies families={[]} />);
  expect(container.textContent).toBe("");
});

test("the_search_block_words_the_stop_runs_time_notes_and_label", () => {
  const doc = cheapest();
  const search = { ...doc.search!, notes: ["db to db.t3.micro refused: too small"] };
  const { container } = render(<PlanSearch search={search} cheapestNote={doc.cheapest_note} />);
  const text = container.textContent ?? "";
  expect(text).toContain("Stopped because it ran out of candidates to try.");
  expect(text).toContain("Runs: 64 of 200 allowed.");
  expect(text).toContain("Time: 31s 800ms used of 1m 15s, the time limit.");
  expect(text).toContain("Refused for size: db to db.t3.micro refused: too small");
  expect(text).toContain("cheapest found within the limit, not proven cheapest");
  cleanup();
  render(<PlanSearch search={{ ...search, stop: "time_limit" }} />);
  expect(
    screen.getByText(/its time limit passed, so cheaper designs may be left untried/),
  ).toBeTruthy();
  cleanup();
  render(<PlanSearch search={{ ...search, stop: "run_cap" }} />);
  expect(
    screen.getByText(/it reached the run cap, so cheaper designs may be left untried/),
  ).toBeTruthy();
});

test("each_part_has_no_axe_violations", async () => {
  const doc = cheapest();
  const parts = [
    () => <PlanSteps steps={done().steps} />,
    () => <PlanComparison comparison={doc.comparison} />,
    () => <PlanComparison comparison={undefined} />,
    () => <PlanFamilies families={done().families} />,
    () => <PlanSearch search={doc.search} cheapestNote={doc.cheapest_note} />,
  ];
  for (const part of parts) {
    const { container } = render(part());
    await expectNoViolations(container);
    cleanup();
  }
});

test("span_agrees_with_the_reports_time_format", () => {
  expect(span(0)).toBe("0s");
  expect(span(800)).toBe("800ms");
  expect(span(31_800)).toBe("31s 800ms");
  expect(span(75_000)).toBe("1m 15s");
  expect(span(3_600_000)).toBe("1h");
  expect(span(3_661_001)).toBe("1h 1m 1s 1ms");
});

test("the_time_limit_says_whose_share_it_is_as_the_report_does", () => {
  const search = cheapest().search!;
  const { container, rerender } = render(<PlanSearch search={search} />);
  expect(container.textContent).toContain("Time: 31s 800ms used of 1m 15s, the time limit.");
  rerender(<PlanSearch search={search} share={{ kind: "search", timeLimitS: 150 }} />);
  expect(container.textContent).toContain(
    "used of 1m 15s, this search's share of the 150 s time limit, which is split between it and each provider section.",
  );
  rerender(<PlanSearch search={search} share={{ kind: "section" }} />);
  expect(container.textContent).toContain(
    "used of 1m 15s, this section's share of the time limit.",
  );
});

test("every_server_string_is_cleaned_not_only_the_names", () => {
  const doc = cheapest();
  const c = doc.comparison!;
  const dirty = (s: string) => `${s}${RLO}`;
  const { container } = render(
    <>
      <PlanComparison
        comparison={{
          ...c,
          conclusion: dirty(c.conclusion),
          not_compared: c.not_compared.map((n) => ({
            provider: dirty(n.provider),
            reason: dirty(n.reason),
          })),
          rows: c.rows.map((r) => ({ ...r, outcome: dirty(r.outcome) })),
        }}
      />
      <PlanFamilies
        families={done().families.map((f) =>
          f.kind === "answer"
            ? { ...f, node: dirty(f.node), notes: f.notes.map(dirty) }
            : { ...f, reason: dirty(f.reason) },
        )}
      />
      <PlanSearch
        search={{ ...doc.search!, notes: [dirty("db refused")] }}
        cheapestNote={dirty(doc.cheapest_note!)}
      />
    </>,
  );
  expect(container.textContent).not.toContain(RLO);
  expect(container.textContent).toContain("db refused");
});
