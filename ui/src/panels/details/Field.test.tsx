import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, test } from "vite-plus/test";
import { BalanceField } from "./Field";

afterEach(cleanup);

const options = () =>
  [...screen.getByLabelText("Balance").querySelectorAll("option")].map((o) => o.textContent);

test("a_component_lists_round_robin_once_as_the_default", () => {
  render(
    <BalanceField
      hint="h"
      value="round-robin"
      defaultLabel="Round robin (default)"
      onChange={() => {}}
    />,
  );
  expect(options()).toEqual([
    "Round robin (default)",
    "Random",
    "Least connections",
    "Power of two choices",
  ]);
  // an explicit round robin in the file reads as the default
  expect((screen.getByLabelText("Balance") as HTMLSelectElement).value).toBe("");
});

test("a_wire_keeps_an_explicit_round_robin_because_it_overrides_the_component", () => {
  render(
    <BalanceField
      hint="h"
      value="round-robin"
      defaultLabel="Same as the component (default)"
      overrides
      onChange={() => {}}
    />,
  );
  expect(options()).toEqual([
    "Same as the component (default)",
    "Round robin",
    "Random",
    "Least connections",
    "Power of two choices",
  ]);
  expect((screen.getByLabelText("Balance") as HTMLSelectElement).value).toBe("round-robin");
});
