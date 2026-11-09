// The response editor's rules (step 8A), apart from the screen: a form's text to the edit the
// server takes, what a route answers now to a form, and the draft patch that stages an edit into
// the file. Pure, so the rules are tested without rendering.

import type { FieldError, PayloadEdit, PayloadPreview } from "../../data/api/payload";
import type { Doc, Op } from "../../data/api/types";

/** The editor's fields as typed. */
export interface Form {
  route: string;
  status: string;
  headers: { name: string; value: string }[];
  body: string;
}

type Parsed = { ok: true; edit: PayloadEdit } | { ok: false; error: FieldError };

const refuse = (field: string, message: string): Parsed => ({
  ok: false,
  error: { field, message },
});

/** The form as an edit, or the field it cannot be one because of. */
export function toEdit(f: Form): Parsed {
  const s = f.status.trim();
  const status = Number(s);
  if (!/^\d+$/.test(s) || status < 200 || status > 599) {
    return refuse("status", "Status must be a whole number from 200 to 599.");
  }
  const text = f.body.trim();
  let body: unknown = null;
  if (text !== "") {
    try {
      body = JSON.parse(text);
    } catch (e) {
      return refuse(
        "body",
        `Body is not valid JSON: ${e instanceof Error ? e.message : String(e)}`,
      );
    }
  }
  if ((status === 204 || status === 304) && body !== null) {
    return refuse("body", `A ${status} reply has no body. Clear the body or pick another status.`);
  }
  const headers: Record<string, string> = {};
  const seen = new Set<string>();
  for (const { name, value } of f.headers) {
    const n = name.trim();
    if (n === "" && value === "") continue; // an empty row is just not filled in
    if (n === "") return refuse("headers", "A header needs a name.");
    if (seen.has(n.toLowerCase())) return refuse("headers", `Header ${n} is listed twice.`);
    seen.add(n.toLowerCase());
    headers[n] = value;
  }
  return { ok: true, edit: { route: f.route, status, body, headers } };
}

const pretty = (body: unknown): string => (body === null ? "" : JSON.stringify(body, null, 2));
const rows = (h: Record<string, string>) =>
  Object.entries(h).map(([name, value]) => ({ name, value }));

/** What a route answers now, as a form to start from. */
export function fromPreview(route: string, p: PayloadPreview): Form {
  return { route, status: String(p.status), headers: rows(p.headers), body: pretty(p.body) };
}

/** An edit the route already has, as a form. */
export function fromEdit(e: PayloadEdit): Form {
  return {
    route: e.route,
    status: String(e.status),
    headers: rows(e.headers),
    body: pretty(e.body),
  };
}

/** `METHOD /path` with the method in capitals and one space, so two spellings compare equal. */
function norm(key: string): string {
  const [method, ...rest] = key.trim().split(/\s+/);
  return `${(method ?? "").toUpperCase()} ${rest.join(" ")}`;
}

/** The key the file spells `route` as, or null when the file has no entry for it. */
export function findRouteKey(endpoints: Record<string, unknown>, route: string): string | null {
  const want = norm(route);
  return Object.keys(endpoints).find((k) => norm(k) === want) ?? null;
}

const isObject = (v: unknown): v is Record<string, unknown> =>
  typeof v === "object" && v !== null && !Array.isArray(v);

/**
 * The draft patch that writes `e` as the route's `override:`. `set` replaces an override that is
 * there; `insert` adds what is missing (the override, the route's entry, or `endpoints:` itself).
 * Not possible when `endpoints:` names another file, which Studio does not edit.
 */
export function stageOps(
  e: PayloadEdit,
  doc: Doc | null,
): { ok: true; ops: Op[] } | { ok: false; reason: string } {
  const override = {
    status: e.status,
    ...(e.body !== null ? { body: e.body } : {}),
    ...(Object.keys(e.headers).length > 0 ? { headers: e.headers } : {}),
  };
  const eps = doc?.endpoints;
  if (typeof eps === "string") {
    return { ok: false, reason: "This route's endpoints are in another file; edit it there." };
  }
  if (!isObject(eps)) {
    return {
      ok: true,
      ops: [{ op: "insert", path: ["endpoints"], value: { [e.route]: { override } } }],
    };
  }
  const key = findRouteKey(eps, e.route);
  if (key === null) {
    return { ok: true, ops: [{ op: "insert", path: ["endpoints", e.route], value: { override } }] };
  }
  const entry = eps[key];
  if (!isObject(entry)) {
    return { ok: true, ops: [{ op: "set", path: ["endpoints", key], value: { override } }] };
  }
  return {
    ok: true,
    ops: [
      {
        op: "override" in entry ? "set" : "insert",
        path: ["endpoints", key, "override"],
        value: override,
      },
    ],
  };
}
