import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { api } from "../../data/api/client";
import { resetPrices } from "../../data/api/prices";
import type { DraftView } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { useMode } from "../../data/store/mode";
import { useRegion } from "../../data/store/region";
import { CostCard } from "./CostCard";

const doc = {
  nodes: {
    app: { type: "service", instance: "aws/m5.large", replicas: 2 },
    gw: { type: "gateway" },
  },
  traffic: { p: { rps: 100, journeys: [] } },
};

beforeEach(() => {
  resetPrices();
  localStorage.clear();
  useRegion.setState({ picked: {}, defaults: {}, regions: {}, ready: false });
  useMode.setState({ mode: "batch" });
  useDraft.setState({ view: { doc } as unknown as DraftView });
  vi.spyOn(api, "getPriceRegions").mockResolvedValue({
    defaults: { aws: "us-east-1" },
    regions: { aws: ["us-east-1", "eu-west-1"] },
  });
  vi.spyOn(api, "getPrices").mockImplementation((_p: string, region: string) =>
    Promise.resolve({
      provider: "aws",
      region,
      prices: [["aws/m5.large", region === "eu-west-1" ? 107_000 : 96_000]],
    }),
  );
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

test("the_card_shows_dollars_a_day_and_opens_to_the_per_node_prices_for_the_days_asked", async () => {
  render(<CostCard />);
  // 2 copies x $0.096 x 24 h = $4.61 a day
  const button = await screen.findByRole("button", { name: /Estimated cost: \$4\.61 a day/ });
  fireEvent.click(button);
  expect(button.getAttribute("aria-expanded")).toBe("true");
  expect(screen.getByText(/An estimate, not a quote/)).toBeTruthy();
  const row = screen.getByText("aws/m5.large").closest("tr") as HTMLElement;
  expect(row.textContent).toMatch(/2.*\$0\.096.*\$4\.61/);
  expect(screen.getAllByText(/no instance type/).length).toBeGreaterThan(0);
  // 30 days by default, then 10: 0.192 x 24 x 10 = $46.08
  fireEvent.change(screen.getByLabelText("Days to estimate"), { target: { value: "10" } });
  expect(screen.getByText("10 days").nextSibling?.textContent).toBe("$46.08");
  // $4.61 a day at 100 requests a second: 0.192 / (3600 x 100) x 1000 = $0.00053 per 1,000
  expect(screen.getByText("Per 1,000 requests").nextSibling?.textContent).toBe("$0.00053");
});

test("another_region_changes_the_price", async () => {
  render(<CostCard />);
  fireEvent.click(await screen.findByRole("button", { name: /Estimated cost/ }));
  await screen.findByText("aws/m5.large");
  fireEvent.change(screen.getByLabelText("AWS region"), { target: { value: "eu-west-1" } });
  // 2 x $0.107 x 24 = $5.14
  await waitFor(() => expect(screen.getByRole("button", { name: /\$5\.14 a day/ })).toBeTruthy());
  expect(useRegion.getState().picked.aws).toBe("eu-west-1");
});
