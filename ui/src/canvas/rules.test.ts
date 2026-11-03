import { afterEach, expect, test } from "vite-plus/test";
import { hasProc } from "../panels/details/fields";
import { mayCall, setKindRules } from "./rules";

afterEach(() => setKindRules(undefined));

test("without_the_servers_table_the_built_in_copy_decides", () => {
  expect(mayCall("gateway", "service")).toBe(true);
  expect(mayCall("gateway", "database")).toBe(false);
  expect(hasProc("service")).toBe(true);
  expect(hasProc("database")).toBe(false);
});

test("the_servers_table_wins_and_a_kind_it_adds_needs_no_change_here", () => {
  setKindRules([
    { kind: "gateway", callees: ["service", "bucket"], service_time: true },
    { kind: "service", callees: ["bucket"], service_time: true },
    { kind: "bucket", callees: [], service_time: false },
  ]);
  // a new kind the page has never heard of: the engine's table is the whole answer
  expect(mayCall("gateway", "bucket")).toBe(true);
  expect(mayCall("service", "bucket")).toBe(true);
  expect(mayCall("bucket", "service")).toBe(false);
  // the server removed gateway -> monolith from the table, so the page follows it
  expect(mayCall("gateway", "monolith")).toBe(false);
  expect(hasProc("bucket")).toBe(false);
  expect(hasProc("gateway")).toBe(true);
});

test("an_unknown_kind_is_allowed_and_the_draft_decides", () => {
  expect(mayCall("mystery", "service")).toBe(true);
  expect(mayCall(undefined, "service")).toBe(true);
});
