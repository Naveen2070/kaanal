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
  /** `""` is JSON; anything else is a media type and the body is sent as typed. */
  contentType: string;
  /** The body is base64 of the bytes to send (a binary body). */
  base64: boolean;
  once: boolean;
}

/** The body kinds the editor offers: the label and the `Content-Type` sent (`""` is JSON). */
export const BODY_KINDS: { label: string; contentType: string; base64: boolean }[] = [
  { label: "JSON", contentType: "", base64: false },
  { label: "Plain text", contentType: "text/plain; charset=utf-8", base64: false },
  { label: "HTML", contentType: "text/html; charset=utf-8", base64: false },
  { label: "XML", contentType: "application/xml", base64: false },
  { label: "CSV", contentType: "text/csv; charset=utf-8", base64: false },
  { label: "Binary (base64)", contentType: "application/octet-stream", base64: true },
];

/** The kind a form is in: the one with its media type, `Binary` for base64, else `Other`. */
export function kindOf(f: Pick<Form, "contentType" | "base64">): string {
  if (f.base64) return "Binary (base64)";
  return BODY_KINDS.find((k) => !k.base64 && k.contentType === f.contentType)?.label ?? "Other";
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
  if (f.contentType !== "") {
    // a text body goes out as typed (spaces and line ends are part of it); base64 may be wrapped
    body = f.base64 ? f.body.replace(/\s+/g, "") : f.body;
  } else if (text !== "") {
    try {
      body = JSON.parse(text);
    } catch (e) {
      return refuse(
        "body",
        `Body is not valid JSON: ${e instanceof Error ? e.message : String(e)}`,
      );
    }
  }
  if ([204, 205, 304].includes(status) && f.contentType !== "" && f.body === "") body = null;
  if ([204, 205, 304].includes(status) && body !== null) {
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
  const edit: PayloadEdit = { route: f.route, status, body, headers };
  if (f.contentType !== "" && body !== null) edit.content_type = f.contentType;
  if (f.base64 && body !== null) edit.encoding = "base64";
  if (f.once) edit.once = true;
  return { ok: true, edit };
}

const pretty = (body: unknown, text: boolean): string =>
  body === null ? "" : text && typeof body === "string" ? body : JSON.stringify(body, null, 2);
const rows = (h: Record<string, string>) =>
  Object.entries(h).map(([name, value]) => ({ name, value }));

/** What a route answers now, as a form to start from. */
export function fromPreview(route: string, p: PayloadPreview): Form {
  const contentType = p.content_type ?? "";
  return {
    base64: p.encoding === "base64",
    route,
    status: String(p.status),
    headers: rows(p.headers),
    body: pretty(p.body, contentType !== ""),
    contentType,
    once: false,
  };
}

/** An edit the route already has, as a form. */
export function fromEdit(e: PayloadEdit): Form {
  return {
    route: e.route,
    status: String(e.status),
    headers: rows(e.headers),
    body: pretty(e.body, Boolean(e.content_type)),
    contentType: e.content_type ?? "",
    base64: e.encoding === "base64",
    once: e.once ?? false,
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

/** Whether the draft's `endpoints:` names another file (the server stages into that one). */
export const endpointsInFile = (doc: Doc | null): boolean => typeof doc?.endpoints === "string";

/**
 * The draft patch that writes `e` as the route's `override:`. `set` replaces an override that is
 * there; `insert` adds what is missing (the override, the route's entry, or `endpoints:` itself).
 * Not possible when `endpoints:` names another file (the draft is only `kaanal.yaml`): the server
 * writes that file itself, see `stagePayloadFile`.
 */
export function stageOps(
  e: PayloadEdit,
  doc: Doc | null,
): { ok: true; ops: Op[] } | { ok: false; reason: string } {
  const override = {
    status: e.status,
    ...(e.body !== null ? { body: e.body } : {}),
    ...(Object.keys(e.headers).length > 0 ? { headers: e.headers } : {}),
    ...(e.content_type ? { content_type: e.content_type } : {}),
    ...(e.encoding ? { encoding: e.encoding } : {}),
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
