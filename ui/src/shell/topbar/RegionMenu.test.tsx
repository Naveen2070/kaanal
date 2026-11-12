import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { api } from "../../data/api/client";
import { resetPrices } from "../../data/api/prices";
import { useRegion } from "../../data/store/region";
import { RegionMenu } from "./RegionMenu";

beforeEach(() => {
  resetPrices();
  localStorage.clear();
  useRegion.setState({ picked: {}, defaults: {}, regions: {}, ready: false });
  vi.spyOn(api, "getPriceRegions").mockResolvedValue({
    defaults: { aws: "us-east-1", azure: "us-east", gcp: "us-central1" },
    regions: {
      aws: ["us-east-1", "eu-west-1"],
      azure: ["us-east", "west-europe"],
      gcp: ["us-central1", "europe-west1"],
    },
  });
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

test("the_globe_opens_the_region_sheet_and_a_choice_is_kept_for_the_page_and_the_browser", async () => {
  render(<RegionMenu />);
  const globe = await screen.findByRole("button", {
    name: /Cloud region for prices and instance types: AWS us-east-1/,
  });
  fireEvent.click(globe);
  expect(globe.getAttribute("aria-expanded")).toBe("true");
  fireEvent.change(screen.getByLabelText("Azure"), { target: { value: "west-europe" } });
  expect(useRegion.getState().picked).toEqual({ azure: "west-europe" });
  expect(JSON.parse(localStorage.getItem("kaanal.regions") ?? "{}")).toEqual({
    azure: "west-europe",
  });
  // the others keep their defaults; Escape closes the sheet
  expect((screen.getByLabelText("AWS") as HTMLSelectElement).value).toBe("us-east-1");
  fireEvent.keyDown(window, { key: "Escape" });
  expect(screen.queryByRole("region", { name: "Cloud region" })).toBeNull();
});
