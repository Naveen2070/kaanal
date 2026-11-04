import { afterEach, expect, test } from "vite-plus/test";
import { hasProc } from "../panels/details/fields";
import { NODE_KINDS } from "./nodeKinds";
import { callsOthers, mayCall, setKindRules } from "./rules";

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

// what GET /__kaanal/draft `kinds` says since the storage kind (the built-in copy must agree)
const SERVER_KINDS = [
  {
    kind: "gateway",
    callees: ["service", "monolith", "auth", "load-balancer"],
    service_time: true,
  },
  {
    kind: "service",
    callees: [
      "service",
      "monolith",
      "database",
      "cache",
      "external-api",
      "auth",
      "load-balancer",
      "storage",
    ],
    service_time: true,
  },
  {
    kind: "monolith",
    callees: [
      "service",
      "monolith",
      "database",
      "cache",
      "external-api",
      "auth",
      "load-balancer",
      "storage",
    ],
    service_time: true,
  },
  { kind: "database", callees: [], service_time: false },
  { kind: "cache", callees: [], service_time: false },
  { kind: "external-api", callees: [], service_time: true },
  {
    kind: "auth",
    callees: [
      "service",
      "monolith",
      "database",
      "cache",
      "external-api",
      "load-balancer",
      "storage",
    ],
    service_time: true,
  },
  {
    kind: "load-balancer",
    callees: [
      "service",
      "monolith",
      "database",
      "external-api",
      "auth",
      "load-balancer",
      "storage",
    ],
    service_time: true,
  },
  { kind: "storage", callees: [], service_time: false },
];

test("storage_is_a_leaf_that_service_monolith_auth_and_balancer_may_call", () => {
  for (const from of ["service", "monolith", "auth", "load-balancer"]) {
    expect(mayCall(from, "storage"), from).toBe(true);
  }
  for (const from of ["gateway", "database", "cache", "external-api", "storage"]) {
    expect(mayCall(from, "storage"), from).toBe(false);
  }
  for (const to of NODE_KINDS.map((k) => k.kind)) expect(mayCall("storage", to), to).toBe(false);
  expect(callsOthers("storage")).toBe(false);
  expect(hasProc("storage")).toBe(false);
});

test("the_built_in_copy_answers_every_pair_as_the_servers_table_does", () => {
  const kinds = NODE_KINDS.map((k) => k.kind);
  expect(SERVER_KINDS.map((k) => k.kind).sort()).toEqual([...kinds].sort());
  const builtIn = new Map<string, boolean>();
  for (const from of kinds) {
    for (const to of kinds) builtIn.set(`${from}>${to}`, mayCall(from, to));
    builtIn.set(`${from}:proc`, hasProc(from));
  }
  setKindRules(SERVER_KINDS);
  for (const from of kinds) {
    for (const to of kinds)
      expect(mayCall(from, to), `${from} to ${to}`).toBe(builtIn.get(`${from}>${to}`));
    expect(hasProc(from), `${from} proc`).toBe(builtIn.get(`${from}:proc`));
  }
});
