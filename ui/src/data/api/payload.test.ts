import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { TOKEN_KEY } from "../../token";
import { mockFetch } from "../../test/fetch";
import { clearPayload, listPayloads, previewPayload, putPayload } from "./payload";

beforeEach(() => sessionStorage.setItem(TOKEN_KEY, "tok"));
afterEach(() => {
  vi.unstubAllGlobals();
  sessionStorage.clear();
});

const edit = { route: "GET /users/{id}", status: 503, body: { a: 1 }, headers: { "x-demo": "1" } };

test("put_sends_the_edit_with_the_token_and_resolves_on_204", async () => {
  const sent = mockFetch([{ status: 204 }]);
  await putPayload(edit);
  expect(sent[0]?.method).toBe("PUT");
  expect(sent[0]?.url).toBe("/__kaanal/payload");
  expect(sent[0]?.headers.Authorization).toBe("Bearer tok");
  expect(sent[0]?.body).toEqual(edit);
});

test("a_422_rejects_with_the_field_and_the_message", async () => {
  mockFetch([
    { status: 422, json: { error: "headers: content-length is set by Kaanal", field: "headers" } },
  ]);
  await expect(putPayload(edit)).rejects.toEqual({
    field: "headers",
    message: "headers: content-length is set by Kaanal",
  });
});

test("another_failure_rejects_with_no_field", async () => {
  mockFetch([{ status: 404, json: { error: "unknown endpoint 'GET /x'" } }]);
  await expect(putPayload(edit)).rejects.toEqual({
    field: null,
    message: "unknown endpoint 'GET /x'",
  });
});

test("clear_encodes_the_route_in_the_query", async () => {
  const sent = mockFetch([{ status: 204 }]);
  await clearPayload("GET /users/{id}");
  expect(sent[0]?.method).toBe("DELETE");
  expect(sent[0]?.url).toBe("/__kaanal/payload?route=GET+%2Fusers%2F%7Bid%7D");
});

test("list_and_preview_read_the_server_shapes", async () => {
  mockFetch([{ json: { edits: [edit] } }, { json: { status: 200, body: { id: 1 }, headers: {} } }]);
  expect(await listPayloads()).toEqual([edit]);
  expect(await previewPayload("GET /users/{id}")).toEqual({
    status: 200,
    body: { id: 1 },
    headers: {},
  });
});
