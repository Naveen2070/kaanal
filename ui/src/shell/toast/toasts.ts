// Notifications (developer, 2026-11-01): every message about the state of things (a run that could
// not start, a refused edit, a connection the draft would not take, a change that cannot be undone)
// is a toast: closeable, stacked, and out of the way of the canvas. Errors and warnings stay until
// closed; success and information go by themselves after a few seconds. The same text is never
// shown twice at once.

import { create } from "zustand";

export type ToastKind = "error" | "warn" | "info" | "success";

export interface Toast {
  id: number;
  kind: ToastKind;
  text: string;
  /** Names the source, so a later message from it replaces this one (or its recovery clears it). */
  key: string;
  /** One button on the toast, such as "Undo". */
  action?: ToastAction;
}

export interface ToastAction {
  label: string;
  run: () => void;
}

/** How long an information or success toast stays; errors and warnings stay until closed. */
export const AUTO_DISMISS_MS = 6000;

interface ToastState {
  toasts: Toast[];
  push: (kind: ToastKind, text: string, key?: string, action?: ToastAction) => void;
  dismiss: (id: number) => void;
  /** Closes the toast a source put up (its recovery). */
  dismissKey: (key: string) => void;
}

let next = 1;

export const useToasts = create<ToastState>((set) => ({
  toasts: [],
  push(kind, text, key = text, action) {
    set((s) => {
      // the same words are already up: say nothing twice
      if (s.toasts.some((t) => t.text === text && t.kind === kind)) return s;
      // a newer message from the same source replaces its older one
      const rest = s.toasts.filter((t) => t.key !== key);
      return { toasts: [...rest, { id: next++, kind, text, key, action }].slice(-5) };
    });
  },
  dismiss: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
  dismissKey: (key) => set((s) => ({ toasts: s.toasts.filter((t) => t.key !== key) })),
}));

/** Outside React: `notify.error("...")`. */
export const notify = {
  error: (text: string, key?: string) => useToasts.getState().push("error", text, key),
  warn: (text: string, key?: string) => useToasts.getState().push("warn", text, key),
  info: (text: string, key?: string, action?: ToastAction) =>
    useToasts.getState().push("info", text, key, action),
  success: (text: string, key?: string) => useToasts.getState().push("success", text, key),
  clear: (key: string) => useToasts.getState().dismissKey(key),
};
