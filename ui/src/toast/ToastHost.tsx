// The toast stack: closeable cards at the top of the free space over the canvas. Each is read out
// when it appears (errors and warnings as alerts, the rest as status), carries its kind in an icon
// and in colour, and has a close button. Information and success close themselves.

import { CircleAlert, CircleCheck, Info, TriangleAlert, X } from "lucide-react";
import { useEffect, useState } from "react";
import { AUTO_DISMISS_MS, useToasts, type Toast, type ToastKind } from "./toasts";

const LOOK: Record<ToastKind, { icon: typeof Info; tone: string }> = {
  error: { icon: CircleAlert, tone: "border-negative text-negative" },
  warn: { icon: TriangleAlert, tone: "border-warn text-warn" },
  info: { icon: Info, tone: "border-border text-muted" },
  success: { icon: CircleCheck, tone: "border-positive text-positive" },
};

function ToastCard({ toast }: { toast: Toast }) {
  const dismiss = useToasts((s) => s.dismiss);
  const { icon: Icon, tone } = LOOK[toast.kind];
  /** Hovered or focused: it stays while someone is reading it or reaching for its button (WCAG 2.2.1). */
  const [held, setHeld] = useState(false);
  useEffect(() => {
    if ((toast.kind !== "info" && toast.kind !== "success") || held) return;
    const t = setTimeout(() => dismiss(toast.id), AUTO_DISMISS_MS);
    return () => clearTimeout(t);
  }, [toast.id, toast.kind, dismiss, held]);
  return (
    <div
      role={toast.kind === "error" || toast.kind === "warn" ? "alert" : "status"}
      onMouseEnter={() => setHeld(true)}
      onMouseLeave={() => setHeld(false)}
      onFocus={() => setHeld(true)}
      onBlur={() => setHeld(false)}
      className={`pointer-events-auto flex w-full max-w-md items-start gap-2.5 rounded-xl border bg-sheet py-2.5 pr-2 pl-3 text-sm shadow-float ${tone}`}
    >
      <Icon size={16} aria-hidden className="mt-0.5 shrink-0" />
      <p className="m-0 min-w-0 flex-1 text-text">{toast.text}</p>
      {toast.action ? (
        <button
          type="button"
          onClick={() => {
            toast.action?.run();
            dismiss(toast.id);
          }}
          className="h-6 shrink-0 rounded-md px-2 text-xs font-semibold text-accent hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
        >
          {toast.action.label}
        </button>
      ) : null}
      <button
        type="button"
        aria-label="Dismiss this notice"
        onClick={() => dismiss(toast.id)}
        className="flex size-6 shrink-0 items-center justify-center rounded-md text-muted hover:bg-raised hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
      >
        <X size={14} aria-hidden />
      </button>
    </div>
  );
}

export function ToastHost() {
  const toasts = useToasts((s) => s.toasts);
  return (
    <section
      aria-label="Notifications"
      className="pointer-events-none flex w-full flex-col items-center gap-2"
    >
      {toasts.map((t) => (
        <ToastCard key={t.id} toast={t} />
      ))}
    </section>
  );
}
