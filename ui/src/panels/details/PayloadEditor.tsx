// The response editor (step 8A, RULES 108): rewrite what one route answers (status, headers, JSON
// body) while a frontend is calling it. Apply now takes effect at once, in memory; Stage in the file
// writes it into the draft's `override:` and the usual Save writes the file. A value the page or
// the server cannot use is said under the field it is about.

import { X } from "lucide-react";
import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import {
  clearPayload,
  listPayloads,
  previewPayload,
  putPayload,
  stagePayloadFile,
  type FieldError,
} from "../../data/api/payload";
import { useAdmin } from "../../data/store/admin";
import { useDraft } from "../../data/store/draft";
import {
  BODY_KINDS,
  endpointsInFile,
  kindOf,
  fromEdit,
  fromPreview,
  stageOps,
  toEdit,
  type Form,
} from "./payloadModel";

const INPUT =
  "h-8 rounded-md border border-control bg-sheet px-2 font-mono text-sm focus-visible:outline-2 focus-visible:outline-accent";
const BTN =
  "h-8 rounded-lg border border-control bg-sheet px-3 text-sm font-medium hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-50";

const message = (e: unknown): string =>
  typeof e === "object" && e !== null && "message" in e
    ? String((e as { message: unknown }).message)
    : String(e);

export function PayloadEditor({ route, onClose }: { route: string; onClose: () => void }) {
  const box = useRef<HTMLDialogElement>(null);
  const first = useRef<HTMLInputElement>(null);
  const errId = useId();
  const doc = useDraft((s) => s.view?.doc ?? null);
  const live = useAdmin((s) => s.liveOnly.includes(route));
  const [form, setForm] = useState<Form | null>(null);
  const [hasEdit, setHasEdit] = useState(false);
  const [error, setError] = useState<FieldError | null>(null);
  // where a staged edit went: "draft", or the endpoints file's name
  const [staged, setStaged] = useState<string | false>(false);
  const [busy, setBusy] = useState(false);

  useLayoutEffect(() => {
    const d = box.current;
    if (!d) return;
    const opener = document.activeElement as HTMLElement | null;
    if (!d.open) {
      if (typeof d.showModal === "function") d.showModal();
      else d.setAttribute("open", "");
    }
    return () => opener?.focus();
  }, []);

  useEffect(() => {
    let gone = false;
    void (async () => {
      try {
        const mine = (await listPayloads()).find((e) => e.route === route);
        const f = mine ? fromEdit(mine) : fromPreview(route, await previewPayload(route));
        if (gone) return;
        setHasEdit(Boolean(mine));
        setForm(f);
      } catch (e) {
        if (!gone) setError({ field: null, message: `Could not load the route: ${message(e)}` });
      }
    })();
    return () => {
      gone = true;
    };
  }, [route]);

  // once the fields exist, focus goes into the dialog
  useEffect(() => {
    if (form) first.current?.focus();
  }, [form === null]); // eslint-disable-line react-hooks/exhaustive-deps

  const stage = form ? stageOps({ route, status: 200, body: null, headers: {} }, doc) : null;
  // a one-shot edit is a live trick: the file would keep it for good
  const inFile = endpointsInFile(doc);
  const stageReason = form?.once
    ? "A one-shot edit is live only."
    : !inFile && stage && !stage.ok
      ? stage.reason
      : null;

  const edit = (patch: Partial<Form>) => {
    setForm((f) => (f ? { ...f, ...patch } : f));
    setStaged(false);
    setError(null);
  };

  const apply = async () => {
    if (!form) return;
    const parsed = toEdit(form);
    if (!parsed.ok) return setError(parsed.error);
    setBusy(true);
    try {
      await putPayload(parsed.edit);
      useAdmin.getState().markLive(route);
      setHasEdit(true);
      setStaged(false);
      setError(null);
    } catch (e) {
      setError(
        typeof e === "object" && e !== null && "field" in e
          ? (e as FieldError)
          : { field: null, message: message(e) },
      );
    } finally {
      setBusy(false);
    }
  };

  const normal = async () => {
    setBusy(true);
    try {
      await clearPayload(route);
    } catch (e) {
      // "has no edit" (404) is the state asked for; anything else leaves the edit in force, and the
      // page keeps saying so
      if (!/has no edit/.test(message(e))) {
        setError({ field: null, message: message(e) });
        setBusy(false);
        return;
      }
    }
    useAdmin.getState().unmarkLive(route);
    setHasEdit(false);
    setStaged(false);
    setBusy(false);
  };

  const stageIt = async () => {
    if (!form) return;
    const parsed = toEdit(form);
    if (!parsed.ok) return setError(parsed.error);
    if (inFile) {
      setBusy(true);
      try {
        const file = await stagePayloadFile(parsed.edit);
        useAdmin.getState().unmarkLive(route);
        setStaged(file);
        setError(null);
      } catch (e) {
        setError(
          typeof e === "object" && e !== null && "field" in e
            ? (e as FieldError)
            : { field: null, message: message(e) },
        );
      } finally {
        setBusy(false);
      }
      return;
    }
    const ops = stageOps(parsed.edit, doc);
    if (!ops.ok) return setError({ field: null, message: ops.reason });
    setBusy(true);
    try {
      if (await useDraft.getState().patch(ops.ops)) {
        useAdmin.getState().unmarkLive(route);
        setStaged("draft");
        setError(null);
      } else {
        setError({ field: null, message: "The draft refused the change." });
      }
    } finally {
      setBusy(false);
    }
  };

  const at = (field: string) => (error?.field === field ? errId : undefined);
  const invalid = (field: string) => (error?.field === field ? true : undefined);

  return (
    <dialog
      ref={box}
      aria-label={`Edit the response of ${route}`}
      aria-modal="true"
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
      className="fixed top-1/2 left-1/2 m-0 flex max-h-[min(40rem,calc(100vh-3rem))] w-[min(36rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 flex-col gap-3 overflow-y-auto rounded-2xl border border-border bg-sheet p-4 text-text shadow-float backdrop:bg-black/40"
    >
      <div className="flex items-center gap-3">
        <h2 className="m-0 min-w-0 truncate font-mono text-base font-semibold">{route}</h2>
        <button
          type="button"
          aria-label="Close"
          onClick={onClose}
          className="ml-auto flex size-8 items-center justify-center rounded-md text-muted hover:bg-raised hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
        >
          <X size={16} aria-hidden />
        </button>
      </div>
      {error && (error.field === null || error.field === "route") ? (
        <p role="alert" id={errId} className="m-0 text-sm text-negative">
          {error.message}
        </p>
      ) : null}
      {form ? (
        <>
          <p className="m-0 text-xs text-muted">
            What this route answers, from now on. Chaos and pins still win, so a route that is down
            keeps failing.
          </p>
          <label className="flex flex-col gap-1 text-sm font-medium">
            Status
            <input
              ref={first}
              name="status"
              inputMode="numeric"
              value={form.status}
              aria-invalid={invalid("status")}
              aria-describedby={at("status")}
              onChange={(e) => edit({ status: e.currentTarget.value })}
              className={`${INPUT} w-24`}
            />
          </label>
          {error?.field === "status" ? (
            <p role="alert" id={errId} className="m-0 text-sm text-negative">
              {error.message}
            </p>
          ) : null}
          <fieldset className="m-0 flex flex-col gap-1.5 border-0 p-0">
            <legend className="mb-1 text-sm font-medium">Headers</legend>
            {form.headers.map((h, i) => (
              <div key={i} className="flex gap-1.5">
                <input
                  aria-label={`Header name ${i + 1}`}
                  aria-invalid={i === 0 ? invalid("headers") : undefined}
                  aria-describedby={i === 0 ? at("headers") : undefined}
                  value={h.name}
                  onChange={(e) =>
                    edit({
                      headers: form.headers.map((x, j) =>
                        j === i ? { ...x, name: e.currentTarget.value } : x,
                      ),
                    })
                  }
                  className={`${INPUT} w-40`}
                />
                <input
                  aria-label={`Header value ${i + 1}`}
                  value={h.value}
                  onChange={(e) =>
                    edit({
                      headers: form.headers.map((x, j) =>
                        j === i ? { ...x, value: e.currentTarget.value } : x,
                      ),
                    })
                  }
                  className={`${INPUT} min-w-0 flex-1`}
                />
                <button
                  type="button"
                  aria-label={`Remove header ${i + 1}`}
                  onClick={() => edit({ headers: form.headers.filter((_, j) => j !== i) })}
                  className="flex size-8 items-center justify-center rounded-md text-muted hover:bg-raised hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
                >
                  <X size={14} aria-hidden />
                </button>
              </div>
            ))}
            <button
              type="button"
              onClick={() => edit({ headers: [...form.headers, { name: "", value: "" }] })}
              className={`${BTN} self-start`}
            >
              Add a header
            </button>
          </fieldset>
          {error?.field === "headers" ? (
            <p role="alert" id={errId} className="m-0 text-sm text-negative">
              {error.message}
            </p>
          ) : null}
          <label className="flex flex-col gap-1 text-sm font-medium">
            Body type
            <select
              name="contentType"
              value={kindOf(form)}
              onChange={(e) => {
                const k = BODY_KINDS.find((x) => x.label === e.currentTarget.value);
                // "Other" keeps the media type typed below
                if (k) edit({ contentType: k.contentType, base64: k.base64 });
                else edit({ base64: false, contentType: form.contentType || "text/plain" });
              }}
              className={`${INPUT} w-48`}
            >
              {BODY_KINDS.map((k) => (
                <option key={k.label} value={k.label}>
                  {k.label}
                </option>
              ))}
              <option value="Other">Other media type</option>
            </select>
          </label>
          {form.contentType !== "" ? (
            <label className="flex flex-col gap-1 text-sm font-medium">
              Content-Type
              <input
                name="mediaType"
                value={form.contentType}
                onChange={(e) => edit({ contentType: e.currentTarget.value })}
                className={INPUT}
              />
            </label>
          ) : null}
          <label className="flex flex-col gap-1 text-sm font-medium">
            {form.contentType === ""
              ? "Body (JSON)"
              : form.base64
                ? "Body (base64 of the bytes)"
                : "Body (text, sent as typed)"}
            <textarea
              name="body"
              rows={8}
              spellCheck={false}
              value={form.body}
              aria-invalid={invalid("body")}
              aria-describedby={at("body")}
              onChange={(e) => edit({ body: e.currentTarget.value })}
              className="rounded-md border border-control bg-sheet p-2 font-mono text-sm focus-visible:outline-2 focus-visible:outline-accent"
            />
          </label>
          {error?.field === "body" ? (
            <p role="alert" id={errId} className="m-0 text-sm text-negative">
              {error.message}
            </p>
          ) : null}
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              name="once"
              checked={form.once}
              onChange={(e) => edit({ once: e.currentTarget.checked })}
            />
            Answer one request only, then back to normal
          </label>
          <p className="m-0 min-h-5 text-xs text-muted" aria-live="polite">
            {live
              ? inFile
                ? "Live only, not in the file. Stage it to write the endpoints file."
                : "Live only, not in the file. Stage it, then Save, to keep it."
              : staged
                ? staged === "draft"
                  ? "In the draft; Save writes the file."
                  : `Written to ${staged}. Apply to run it from the file.`
                : hasEdit
                  ? "Live."
                  : ""}
          </p>
          {stageReason ? <p className="m-0 text-xs text-muted">{stageReason}</p> : null}
          <div className="flex flex-wrap gap-2">
            <button type="button" disabled={busy} onClick={() => void apply()} className={BTN}>
              Apply now
            </button>
            <button
              type="button"
              disabled={busy || !hasEdit}
              onClick={() => void normal()}
              className={BTN}
            >
              Back to normal
            </button>
            <button
              type="button"
              disabled={busy || stageReason !== null}
              onClick={() => void stageIt()}
              className={BTN}
            >
              Stage in the file
            </button>
          </div>
        </>
      ) : (
        <p className="m-0 text-sm text-muted" aria-live="polite">
          {error ? "" : "Loading…"}
        </p>
      )}
    </dialog>
  );
}
