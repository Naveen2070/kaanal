import { act, cleanup, render } from "@testing-library/react";
import { NuqsAdapter } from "nuqs/adapters/react";
import { useEffect } from "react";
import { afterEach, beforeEach, expect, test } from "vite-plus/test";
import { TOKEN_KEY } from "../token";
import { useSelected, useTab } from "./view";

let setSelected: ReturnType<typeof useSelected>[1];
let setTab: ReturnType<typeof useTab>[1];

function Probe() {
  const [, select] = useSelected();
  const [, tab] = useTab();
  // handed to the tests from an effect: assigning during render is a side effect
  useEffect(() => {
    setSelected = select;
    setTab = tab;
  });
  return null;
}

beforeEach(() => {
  sessionStorage.setItem(TOKEN_KEY, "secret-token-value");
  history.replaceState(null, "", "/__kaanal/studio");
});
afterEach(() => {
  cleanup();
  sessionStorage.clear();
});

test("nuqs_state_never_contains_the_token", async () => {
  render(
    <NuqsAdapter>
      <Probe />
    </NuqsAdapter>,
  );
  await act(async () => {
    await setSelected("db");
    await setTab("log");
  });
  expect(location.search).toContain("node=db");
  expect(location.search).toContain("tab=log");
  expect(location.search + location.hash).not.toContain("secret-token-value");
});

test("a_node_name_with_hash_slash_space_and_unicode_survives_the_address_bar", async () => {
  const name = "a#b/c d é日本";
  render(
    <NuqsAdapter>
      <Probe />
    </NuqsAdapter>,
  );
  await act(async () => {
    await setSelected(name);
  });
  expect(new URLSearchParams(location.search).get("node")).toBe(name);
  expect(location.hash).toBe("");
});
