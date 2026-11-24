// The sizing plan on screen: it starts one, polls its record while it runs (the steps arrive as the
// walk takes them), and fetches the Markdown report once when it is done. It lives outside the
// dialog, so closing the dialog neither stops nor loses a plan. No React here; the timer is a
// module-level handle so `reset` can stop it.

import { create } from "zustand";
import { api } from "../api/client";
import type { PlanBody, PlanRecord } from "../api/types.plan";
import { DEFAULT_FORM, type FormState } from "../../panels/plan/planMath";

export type PlanStatus = "idle" | "running" | "done" | "failed" | "busy";

export interface PlanState {
  status: PlanStatus;
  record: PlanRecord | null;
  /** The Markdown report, fetched once when the plan is done. */
  report: string | null;
  /** Why it failed or could not start; with `done`, why the report could not be fetched. */
  error: string | null;
  /** What the plan on screen was started with, and the server process (`DraftView.boot`) it went to. */
  sent: (PlanBody & { boot: string | null }) | null;
  /** The dialog is open. */
  open: boolean;
  /** What the form holds; kept here because the dialog unmounts when closed. `reset` keeps it. */
  form: FormState;
  start: (body: PlanBody, boot?: string | null) => Promise<void>;
  reset: () => void;
  setOpen: (open: boolean) => void;
  setForm: (form: FormState) => void;
}

const POLL_MS = 500;
/** Consecutive failed polls tolerated before the plan counts as failed. */
const MAX_POLL_ERRORS = 3;

const idle = {
  status: "idle" as PlanStatus,
  record: null,
  report: null,
  error: null,
  sent: null,
};

let timer: ReturnType<typeof setTimeout> | undefined;
/** Bumped by every start and reset; a poll from an older one drops its answer. */
let epoch = 0;

function stop() {
  clearTimeout(timer);
  timer = undefined;
  epoch++;
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

export const usePlan = create<PlanState>((set, get) => {
  async function poll(id: string, mine: number, errors = 0) {
    try {
      // the same route serves runs, sweeps and plans; the caller started a plan
      const rec = (await api.getRun(id)) as unknown as PlanRecord;
      if (mine !== epoch) return;
      if (rec.state === "running") {
        set({ record: rec });
        timer = setTimeout(() => void poll(id, mine, 0), POLL_MS);
      } else if (rec.state === "failed") {
        set({ status: "failed", record: rec, error: rec.error ?? "the plan failed" });
      } else {
        set({ record: rec });
        let report: string | null = null;
        let error: string | null = null;
        try {
          report = await api.getPlanReport(id);
        } catch (e) {
          error = message(e);
        }
        if (mine === epoch) set({ status: "done", report, error });
      }
    } catch (e) {
      if (mine !== epoch) return;
      if (errors < MAX_POLL_ERRORS) {
        // a blip must not end a healthy plan; keep what is on screen and try again
        timer = setTimeout(() => void poll(id, mine, errors + 1), POLL_MS);
      } else set({ status: "failed", error: message(e) });
    }
  }

  return {
    ...idle,
    open: false,
    form: DEFAULT_FORM,
    // Resolves after the first poll; later polls run on the timer.
    async start(body, boot = null) {
      // a plan on its way is not dropped by a second one (the server would answer busy and the first
      // plan's answer would be lost); `reset` ends it first
      if (get().status === "running") return;
      stop();
      const mine = epoch;
      set({ ...idle, status: "running", sent: { ...body, boot } });
      try {
        const r = await api.startPlan(body);
        if (mine !== epoch) return;
        if (r.kind === "busy") set({ status: "busy", error: r.message });
        else await poll(r.id, mine);
      } catch (e) {
        if (mine === epoch) set({ status: "failed", error: message(e) });
      }
    },
    reset() {
      stop();
      set({ ...idle });
    },
    setOpen(open) {
      set({ open });
    },
    setForm(form) {
      set({ form });
    },
  };
});
