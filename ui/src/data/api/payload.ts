// Live response edits (step 8A, RULES 108): a route answers a status, headers and a JSON or text body the
// person chose, from now on, with no Apply. The server refuses what it cannot serve and names the
// field, so a form can show the message under it.

import { ApiError, api } from "./client";

export interface PayloadEdit {
  /** The route key, as `GET /users/{id}`. */
  route: string;
  status: number;
  body: unknown;
  headers: Record<string, string>;
  /** A text body is sent as is with this `Content-Type`; left out, the body is JSON. */
  content_type?: string | null;
  /** `base64`: the string `body` is base64 of the bytes to send (needs `content_type`). */
  encoding?: "base64" | null;
  /** Answers one request, then is gone. */
  once?: boolean;
}

/** What a route answers with no edit. */
export interface PayloadPreview {
  status: number;
  body: unknown;
  headers: Record<string, string>;
  content_type?: string | null;
  encoding?: "base64" | null;
}

/** Why an edit was refused: the field it is about (`status`, `body`, `headers`, `route`) or none. */
export interface FieldError {
  field: string | null;
  message: string;
}

function asFieldError(e: unknown): never {
  if (e instanceof ApiError) {
    const field = (e.body as { field?: unknown } | undefined)?.field;
    throw {
      field: typeof field === "string" ? field : null,
      message: e.message,
    } satisfies FieldError;
  }
  throw e;
}

export async function listPayloads(): Promise<PayloadEdit[]> {
  return (await api.getPayloads()).edits;
}

/** Sets the edit; rejects with a `FieldError`. */
export async function putPayload(edit: PayloadEdit): Promise<void> {
  try {
    await api.putPayload(edit);
  } catch (e) {
    asFieldError(e);
  }
}

/**
 * Writes the edit into the endpoints file the draft names, at once and with no Save (that file is
 * not part of the draft); the system runs it after the next Apply. Resolves to the file's name;
 * rejects with a `FieldError`.
 */
export async function stagePayloadFile(edit: PayloadEdit): Promise<string> {
  try {
    return (await api.stagePayloadFile(edit)).file;
  } catch (e) {
    asFieldError(e);
  }
}

/** Back to normal for one route; rejects with a `FieldError` (404 when it had no edit). */
export async function clearPayload(route: string): Promise<void> {
  try {
    await api.deletePayload(route);
  } catch (e) {
    asFieldError(e);
  }
}

export function previewPayload(route: string): Promise<PayloadPreview> {
  return api.previewPayload(route);
}
