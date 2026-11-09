import { expect, test } from "vite-plus/test";
import type { Doc } from "../../data/api/types";
import { findRouteKey, fromEdit, fromPreview, stageOps, toEdit, type Form } from "./payloadModel";

const form = (over: Partial<Form> = {}): Form => ({
  route: "GET /users/{id}",
  status: "503",
  headers: [{ name: "x-demo", value: "1" }],
  body: '{ "down": true }',
  ...over,
});

test("toEdit_parses_status_headers_and_a_json_body", () => {
  expect(toEdit(form())).toEqual({
    ok: true,
    edit: {
      route: "GET /users/{id}",
      status: 503,
      body: { down: true },
      headers: { "x-demo": "1" },
    },
  });
});

test("a_blank_body_is_no_body_and_a_blank_header_row_is_skipped", () => {
  const r = toEdit(form({ body: "  ", headers: [{ name: "", value: "" }] }));
  expect(r).toEqual({
    ok: true,
    edit: { route: "GET /users/{id}", status: 503, body: null, headers: {} },
  });
});

test.each([["99"], ["600"], ["abc"], ["20.5"], [""]])("toEdit_rejects_status_%s_on_status", (s) => {
  const r = toEdit(form({ status: s }));
  expect(r.ok).toBe(false);
  if (!r.ok) expect(r.error.field).toBe("status");
});

test("toEdit_rejects_a_body_on_204_and_304_but_not_a_blank_one", () => {
  for (const s of ["204", "304"]) {
    const bad = toEdit(form({ status: s }));
    expect(bad.ok === false && bad.error.field).toBe("body");
    expect(toEdit(form({ status: s, body: "" })).ok).toBe(true);
    expect(toEdit(form({ status: s, body: "null" })).ok).toBe(true);
  }
});

test("toEdit_reports_invalid_json_on_body_with_the_parser_message", () => {
  const r = toEdit(form({ body: "{ nope" }));
  expect(r.ok).toBe(false);
  if (!r.ok) {
    expect(r.error.field).toBe("body");
    expect(r.error.message).toMatch(/^Body is not valid JSON/);
  }
});

test("toEdit_rejects_a_duplicate_header_name_whatever_its_case", () => {
  const r = toEdit(
    form({
      headers: [
        { name: "X-Demo", value: "1" },
        { name: "x-demo", value: "2" },
      ],
    }),
  );
  expect(r.ok === false && r.error.field).toBe("headers");
});

test("toEdit_rejects_a_value_with_no_name", () => {
  const r = toEdit(form({ headers: [{ name: "", value: "x" }] }));
  expect(r.ok === false && r.error.field).toBe("headers");
});

test("fromPreview_pretty_prints_the_body_and_lists_the_headers", () => {
  const f = fromPreview("GET /x", { status: 200, body: { a: 1 }, headers: { "x-a": "b" } });
  expect(f).toEqual({
    route: "GET /x",
    status: "200",
    headers: [{ name: "x-a", value: "b" }],
    body: '{\n  "a": 1\n}',
  });
  expect(fromPreview("GET /x", { status: 204, body: null, headers: {} }).body).toBe("");
  expect(fromEdit({ route: "GET /x", status: 503, body: { e: 1 }, headers: {} }).status).toBe(
    "503",
  );
});

const edit = { route: "GET /users/{id}", status: 503, body: { a: 1 }, headers: {} };

test("stageOps_sets_an_override_that_is_already_there", () => {
  const doc = {
    endpoints: { "GET /users/{id}": { override: { status: 200 } } },
  } as unknown as Doc;
  expect(stageOps(edit, doc)).toEqual({
    ok: true,
    ops: [
      {
        op: "set",
        path: ["endpoints", "GET /users/{id}", "override"],
        value: { status: 503, body: { a: 1 } },
      },
    ],
  });
});

test("stageOps_inserts_the_override_into_an_entry_the_route_has", () => {
  const doc = { endpoints: { "GET /users/{id}": { chain: ["db"] } } } as unknown as Doc;
  const r = stageOps({ ...edit, headers: { "x-a": "b" } }, doc);
  expect(r).toEqual({
    ok: true,
    ops: [
      {
        op: "insert",
        path: ["endpoints", "GET /users/{id}", "override"],
        value: { status: 503, body: { a: 1 }, headers: { "x-a": "b" } },
      },
    ],
  });
});

test("stageOps_inserts_the_entry_when_the_route_has_none_and_the_map_when_there_is_none", () => {
  const withMap = { endpoints: { "GET /other": {} } } as unknown as Doc;
  expect(stageOps(edit, withMap)).toEqual({
    ok: true,
    ops: [
      {
        op: "insert",
        path: ["endpoints", "GET /users/{id}"],
        value: { override: { status: 503, body: { a: 1 } } },
      },
    ],
  });
  expect(stageOps(edit, {} as Doc)).toEqual({
    ok: true,
    ops: [
      {
        op: "insert",
        path: ["endpoints"],
        value: { "GET /users/{id}": { override: { status: 503, body: { a: 1 } } } },
      },
    ],
  });
});

test("stageOps_refuses_when_endpoints_is_a_file_reference", () => {
  const r = stageOps(edit, { endpoints: "endpoints.yaml" } as unknown as Doc);
  expect(r.ok).toBe(false);
  if (!r.ok) expect(r.reason).toMatch(/another file/);
});

test("the_route_key_is_found_whatever_its_spelling_in_the_file", () => {
  const eps = { "get /users/{id}": {}, "POST  /orders": {} };
  expect(findRouteKey(eps, "GET /users/{id}")).toBe("get /users/{id}");
  expect(findRouteKey(eps, "POST /orders")).toBe("POST  /orders");
  expect(findRouteKey(eps, "GET /nope")).toBeNull();
});

test("stageOps_writes_to_the_key_as_the_file_spells_it", () => {
  const doc = { endpoints: { "get /users/{id}": { chain: ["db"] } } } as unknown as Doc;
  const r = stageOps(edit, doc);
  expect(r.ok && (r.ops[0] as { path: unknown }).path).toEqual([
    "endpoints",
    "get /users/{id}",
    "override",
  ]);
});
