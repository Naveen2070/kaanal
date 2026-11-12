import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { api } from "../../data/api/client";
import { resetPrices } from "../../data/api/prices";
import type { DraftView } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { fileRegions, regionOp, useRegion } from "../../data/store/region";
import { RegionMenu } from "./RegionMenu";

beforeEach(() => {
  resetPrices();
  localStorage.clear();
  useRegion.setState({ picked: {}, fromFile: {}, defaults: {}, regions: {}, ready: false });
  useDraft.setState({ view: null } as never);
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

test("the_files_region_wins_and_a_choice_is_written_into_the_file", async () => {
  const patch = vi.fn(() => Promise.resolve(true));
  useDraft.setState({
    view: { doc: { nodes: {}, region: { aws: "eu-west-1" } } } as unknown as DraftView,
    patch,
  } as never);
  useRegion.setState({ picked: { aws: "us-east-1" } });
  render(<RegionMenu />);
  const globe = await screen.findByRole("button", { name: /AWS eu-west-1/ });
  fireEvent.click(globe);
  expect((screen.getByLabelText("AWS") as HTMLSelectElement).value).toBe("eu-west-1");
  // a provider the file leaves out is inserted next to the one it has
  fireEvent.change(screen.getByLabelText("Azure"), { target: { value: "west-europe" } });
  expect(patch).toHaveBeenCalledWith([
    { op: "insert", path: ["region", "azure"], value: "west-europe" },
  ]);
  fireEvent.change(screen.getByLabelText("AWS"), { target: { value: "us-east-1" } });
  expect(patch).toHaveBeenLastCalledWith([
    { op: "set", path: ["region", "aws"], value: "us-east-1" },
  ]);
});

test("the_whole_mapping_is_inserted_when_the_file_has_none", () => {
  expect(regionOp({ nodes: {} }, "gcp", "europe-west1")).toEqual({
    op: "insert",
    path: ["region"],
    value: { gcp: "europe-west1" },
  });
  expect(regionOp(null, "aws", "eu-west-1").op).toBe("insert");
  expect(fileRegions({ region: { aws: "eu-west-1", gcp: 7 } })).toEqual({ aws: "eu-west-1" });
  expect(fileRegions({ region: ["aws"] })).toEqual({});
});
