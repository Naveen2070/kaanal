import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, test } from "vite-plus/test";
import { BalanceField, NumberField } from "./Field";

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

test("a_quiet_hint_stays_for_screen_readers_but_leaves_the_screen_for_the_help_button", () => {
  render(
    <NumberField
      label="Bandwidth"
      help="bandwidth"
      quietHint
      hint="What the link carries each second."
      value={null}
      onCommit={() => {}}
    />,
  );
  expect(screen.getByRole("button", { name: "About Bandwidth" })).toBeTruthy();
  const box = screen.getByLabelText("Bandwidth");
  const hint = document.getElementById(box.getAttribute("aria-describedby") ?? "");
  expect(hint?.textContent).toBe("What the link carries each second.");
  expect(hint?.className).toContain("sr-only");
});

test("without_quiet_the_hint_is_still_shown_beside_the_help_button", () => {
  render(
    <NumberField label="Service time" help="time" hint="Shown." value={1} onCommit={() => {}} />,
  );
  const hint = screen.getByText("Shown.");
  expect(hint.className).not.toContain("sr-only");
});

test("a_balance_field_can_carry_a_help_button_and_keeps_the_live_mock_caveat_visible", () => {
  render(
    <BalanceField
      hint="h"
      help="balance"
      quietHint
      value="p2c"
      defaultLabel="Round robin (default)"
      onChange={() => {}}
    />,
  );
  expect(screen.getByRole("button", { name: "About Balance" })).toBeTruthy();
  // the caveat is about this choice, not about the word, so it is never hidden
  expect(screen.getByText(/the live mock plays this as round robin/).className).not.toContain(
    "sr-only",
  );
});
