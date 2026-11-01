// Apply and Save, the two buttons that touch something outside the draft. The run loop never
// applies (the mocks keep serving the applied system while you draw); only these do, and only on
// a click. Both first wait for every edit still on its way to the server, so what they act on is
// what the screen shows. A result opens in a modal dialog: what Apply kept and dropped, what Save
// would lose, a draft that would not start, a conflict with the file on disk, or a refusal.

import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { ApiError, api } from "../api/client";
import type { ApplyResult, SaveResult, ValidateResult } from "../api/types";
import { useDraft } from "../store/draft";

type Dialog =
  | { kind: "applied"; result: ApplyResult }
  | { kind: "saved"; result: SaveResult }
  | { kind: "lost"; lost: string[] }
  | { kind: "broken"; problems: string[] }
  | { kind: "conflict"; message: string }
  | { kind: "error"; title: string; message: string };

interface ErrBody {
  lost?: string[];
  conflict?: boolean;
  diagnostics?: { message?: string }[];
}

function Modal({
  title,
  onClose,
  children,
}: {
  title: string;
  onClose: () => void;
  children: React.ReactNode;
}) {
  const box = useRef<HTMLDialogElement>(null);
  // a layout effect: the dialog is open in the commit that adds it, so nothing (a screen reader, a
  // role query) sees it closed for a tick
  useLayoutEffect(() => {
    const d = box.current;
    if (!d) return;
    // when the dialog goes, focus goes back to the button that opened it
    const opener = document.activeElement as HTMLElement | null;
    if (!d.open) {
      if (typeof d.showModal === "function") d.showModal();
      else d.setAttribute("open", "");
    }
    return () => opener?.focus();
  }, []);
  return (
    <dialog
      ref={box}
      aria-label={title}
      aria-modal="true"
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
      onKeyDown={(e) => e.key === "Escape" && onClose()}
      className="w-full max-w-md rounded-lg border border-border bg-sheet p-4 text-sm text-text shadow-xl backdrop:bg-black/40"
    >
      <h2 className="mb-2 text-base font-semibold">{title}</h2>
      {children}
    </dialog>
  );
}

/** Copies `text`; when the browser will not, the text is shown to select by hand. */
function CopyDraft({ text }: { text: string }) {
  const [state, setState] = useState<"idle" | "copied" | "manual">("idle");
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setState("copied");
    } catch {
      setState("manual");
    }
  };
  return (
    <>
      <button
        type="button"
        className="rounded border border-control bg-sheet text-text px-2"
        onClick={() => void copy()}
      >
        {state === "copied" ? "Copied" : "Copy my draft"}
      </button>
      {state === "manual" ? (
        <label className="mt-2 block text-xs">
          Copying was blocked. Select this text and copy it yourself:
          <textarea
            readOnly
            value={text}
            rows={6}
            onFocus={(e) => e.currentTarget.select()}
            className="mt-1 w-full rounded border p-1 font-mono"
          />
        </label>
      ) : null}
    </>
  );
}

const names = (xs: string[]) => (xs.length === 0 ? "nothing" : xs.join(", "));

/** What would stop `kaanal up` from starting on the draft as it is. */
function problemsOf(v: ValidateResult): string[] {
  return [
    ...v.diagnostics.map((d) => d.message),
    ...v.flagged.map((f) => `scenario file ${f.file}: ${f.message}`),
  ];
}

/**
 * Apply and Save, with `lead` (Run) before them on one row; below it, `note` (what the loop is
 * doing) over the draft's state.
 */
export function SaveFlow({ lead, note }: { lead?: ReactNode; note?: ReactNode }) {
  const view = useDraft((s) => s.view);
  const load = useDraft((s) => s.load);
  const settled = useDraft((s) => s.settled);
  const [busy, setBusy] = useState<"apply" | "save" | null>(null);
  const [dialog, setDialog] = useState<Dialog | null>(null);

  const apply = async () => {
    setBusy("apply");
    try {
      await settled();
      const result = await api.apply();
      setDialog({ kind: "applied", result });
      await load();
    } catch (e) {
      const body = e instanceof ApiError ? (e.body as ErrBody | null) : null;
      setDialog({
        kind: "error",
        title: "Apply did not happen",
        message: body?.diagnostics?.[0]?.message ?? (e instanceof Error ? e.message : String(e)),
      });
    } finally {
      setBusy(null);
    }
  };

  /** `check`: first ask whether the file would start; `confirmLoss`: the comment-loss confirmation. */
  const save = async ({ check, confirmLoss }: { check: boolean; confirmLoss: boolean }) => {
    setBusy("save");
    try {
      await settled();
      if (check) {
        // a draft that parses can still be a project that does not start: say so before writing it
        const problems = problemsOf(await api.validate());
        if (problems.length > 0) {
          setDialog({ kind: "broken", problems });
          return;
        }
      }
      const result = await api.save(confirmLoss ? { confirm_loss: true } : {});
      setDialog({ kind: "saved", result });
      await load();
    } catch (e) {
      const body = e instanceof ApiError ? (e.body as ErrBody | null) : null;
      if (e instanceof ApiError && e.status === 409 && body?.lost) {
        setDialog({ kind: "lost", lost: body.lost });
      } else if (e instanceof ApiError && e.status === 409 && body?.conflict) {
        setDialog({ kind: "conflict", message: e.message });
      } else {
        // the draft is untouched: a failed save loses nothing
        setDialog({
          kind: "error",
          title: "Save did not happen",
          message: e instanceof Error ? e.message : String(e),
        });
      }
    } finally {
      setBusy(null);
    }
  };

  const close = () => setDialog(null);
  const dirty = view?.dirty ?? false;

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        {lead}
        <button
          type="button"
          data-command="apply"
          disabled={busy !== null || view?.doc == null}
          onClick={() => void apply()}
          title="Make the draft the running system (the mocks switch to it)"
          className="h-8 rounded-lg border border-control bg-sheet px-3 font-medium text-text hover:bg-raised disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-accent"
        >
          Apply
        </button>
        <button
          type="button"
          data-tour="save"
          data-command="save"
          disabled={busy !== null || view?.doc == null}
          onClick={() => void save({ check: true, confirmLoss: false })}
          title="Write the draft to kaanal.yaml"
          className="h-8 rounded-lg bg-primary px-3.5 font-medium text-on-primary hover:opacity-90 disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
        >
          Save
        </button>
      </div>
      <div className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 flex-nowrap items-center gap-x-3 whitespace-nowrap">
          {note}
          <span
            className={`flex items-start gap-1.5 text-xs ${dirty ? "text-warn" : "text-muted"}`}
          >
            <span
              aria-hidden
              className={`mt-1 size-1.5 shrink-0 rounded-full ${dirty ? "bg-warn-fill" : "bg-ok-fill"}`}
            />
            {`Draft: ${dirty ? "not applied" : "applied"} · ${view?.saved === false ? "not saved" : "saved"}`}
          </span>
        </div>
      </div>

      {dialog?.kind === "applied" ? (
        <Modal title="Applied" onClose={close}>
          <p>The draft is now the running system (generation {dialog.result.generation}).</p>
          <ul className="my-2 text-xs">
            <li>Kept: {kept(dialog.result.kept)}</li>
            <li>Dropped: {dropped(dialog.result.dropped)}</li>
            {dialog.result.flagged.length > 0 ? (
              <li>
                Scenario files left out:{" "}
                {dialog.result.flagged.map((f) => `${f.scenario} (${f.message})`).join("; ")}
              </li>
            ) : null}
          </ul>
          <button
            type="button"
            onClick={close}
            className="rounded border border-control bg-sheet text-text px-2"
          >
            Close
          </button>
        </Modal>
      ) : null}

      {dialog?.kind === "saved" ? (
        <Modal title="Saved" onClose={close}>
          <p>Written: {names(dialog.result.written)}.</p>
          {dialog.result.removed.length > 0 ? (
            <p>Removed: {names(dialog.result.removed)}.</p>
          ) : null}
          <p className="text-xs text-muted">
            {dialog.result.layout_written
              ? "Positions saved beside it (kaanal.layout.json)."
              : (dialog.result.layout_note ?? "Positions were not changed.")}
          </p>
          <button
            type="button"
            onClick={close}
            className="mt-2 rounded border border-control bg-sheet text-text px-2"
          >
            Close
          </button>
        </Modal>
      ) : null}

      {dialog?.kind === "broken" ? (
        <Modal title="This file would not start" onClose={close}>
          <p className="mb-1">
            The draft does not pass the same checks <code>kaanal up</code> runs:
          </p>
          <ul className="mb-2 list-disc pl-5 text-xs">
            {dialog.problems.map((p, i) => (
              <li key={i}>{p}</li>
            ))}
          </ul>
          <div className="flex gap-2">
            <button
              type="button"
              className="rounded border border-negative px-2 text-negative"
              onClick={() => {
                close();
                void save({ check: false, confirmLoss: false });
              }}
            >
              Save anyway
            </button>
            <button
              type="button"
              onClick={close}
              className="rounded border border-control bg-sheet text-text px-2"
            >
              Cancel
            </button>
          </div>
        </Modal>
      ) : null}

      {dialog?.kind === "lost" ? (
        <Modal title="Saving loses some of the file" onClose={close}>
          <p className="mb-1">Edits from here could not keep these, so saving drops them:</p>
          <ul className="mb-2 list-disc pl-5 text-xs">
            {dialog.lost.map((l, i) => (
              <li key={i}>{l}</li>
            ))}
          </ul>
          <div className="flex gap-2">
            <button
              type="button"
              className="rounded border border-negative px-2 text-negative"
              onClick={() => {
                close();
                void save({ check: false, confirmLoss: true });
              }}
            >
              Save anyway
            </button>
            <button
              type="button"
              onClick={close}
              className="rounded border border-control bg-sheet text-text px-2"
            >
              Cancel
            </button>
          </div>
        </Modal>
      ) : null}

      {dialog?.kind === "conflict" ? (
        <Modal title="The file changed on disk" onClose={close}>
          <p>{dialog.message}</p>
          <p className="my-2 text-xs text-muted">
            Nothing was written and your draft is still here. Studio cannot read the file again
            while it runs: copy your draft first, then stop and restart <code>kaanal studio</code>{" "}
            (the draft in memory is lost on restart) and paste your changes into the file as it is
            now.
          </p>
          <div className="flex flex-wrap items-start gap-2">
            <CopyDraft text={view?.text ?? ""} />
            <button
              type="button"
              onClick={close}
              className="rounded border border-control bg-sheet text-text px-2"
            >
              Close
            </button>
          </div>
        </Modal>
      ) : null}

      {dialog?.kind === "error" ? (
        <Modal title={dialog.title} onClose={close}>
          <p role="alert">{dialog.message}</p>
          <p className="my-2 text-xs text-muted">The draft is unchanged.</p>
          <button
            type="button"
            onClick={close}
            className="rounded border border-control bg-sheet text-text px-2"
          >
            Close
          </button>
        </Modal>
      ) : null}
    </div>
  );
}

function kept(k: Record<string, unknown>): string {
  const parts = Object.entries(k).flatMap(([what, v]) => {
    const n = Array.isArray(v) ? v.length : typeof v === "number" ? v : 0;
    return n > 0 ? [`${n} ${what.replaceAll("_", " ")}`] : [];
  });
  return parts.length === 0 ? "nothing to carry over" : parts.join(", ");
}

function dropped(d: Record<string, string[]>): string {
  const parts = Object.entries(d).flatMap(([what, v]) =>
    v.length > 0 ? [`${what.replaceAll("_", " ")}: ${v.join(", ")}`] : [],
  );
  return parts.length === 0 ? "nothing" : parts.join("; ");
}
