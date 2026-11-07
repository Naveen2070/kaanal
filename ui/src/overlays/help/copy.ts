// The words behind every jargon label (spec 5.2). The label is what the screen shows; this is what
// its help popover says: what it is (one plain sentence, under 25 words), then, where it helps, an
// example with figures worked from the formula, a hint on when to change it, and the glossary entry
// the "Open in the glossary" link goes to (the entry whose id is the help id unless `entry` names
// another).

import { RESULTS_HELP } from "./copyResults";
import { WIRE_HELP } from "./copyWire";

export interface Help {
  /** What the help button is named: "About Time". */
  label: string;
  what: string;
  /** A longer explanation for the glossary, after `what`. */
  more?: string;
  /** What it stands for in a real system, for the glossary. */
  real?: string;
  /** Figures worked from the model, never invented (RULES 66). */
  example?: string;
  /** When to change it, in one line. */
  hint?: string;
  /** The glossary entry to open; the help id itself when absent. */
  entry?: string;
}

const BASE = {
  time: {
    label: "Time",
    what: "How long one request holds a slot while it is being worked on. Longer means fewer requests finished each second.",
    example: "50 ms a request on 1 slot finishes at most 20 requests a second.",
    hint: "Raise it to model a slower service; the throughput limit follows.",
  },
  slots: {
    label: "Slots",
    what: "How many requests one copy works on at the same moment. More slots, more throughput, until the time per request limits it.",
    example: "32 slots at 25 ms each finish at most 32 / 0.025 = 1,280 requests a second.",
    hint: "Match it to a thread pool, a worker count or a connection pool.",
  },
  copies: {
    label: "Copies",
    what: "Identical copies of this component, each with its own slots and its own queue. Load is shared between them.",
    example: "3 copies with 32 slots each work on up to 96 requests at once.",
    hint: "Add copies to scale out; put a load balancer in front to share the load.",
  },
  p99: {
    label: "p99",
    what: "99 in every 100 requests finish faster than this. It shows the slow tail that an average hides.",
    example: "A p99 of 300 ms means 1 request in 100 took longer than 300 ms.",
    hint: "Watch it rather than the average: it moves first when a queue builds up.",
  },
  goodput: {
    label: "Goodput",
    what: "Requests that finished successfully each second, not counting failed or turned-away ones.",
    example: "120 requests a second arrive and 20 fail: goodput is 100 a second.",
    hint: "When it stays far below the offered load, something is failing or queuing.",
  },
  saturation: {
    label: "Breaks first",
    what: "The point where a component has no spare capacity. Past it, requests queue up and then get turned away.",
    example: "A component that finishes 210 requests a second saturates at about 210 a second.",
    hint: "The first component to reach it is the one named under Breaks first.",
  },
  arriving: {
    label: "Arriving",
    what: "Requests a second that got in and were worked on here. Turned-away ones are not counted.",
    example: "A gateway sends 120 a second and this component turns 20 away: Arriving shows 100/s.",
    hint: "It can read lower than the component calling it, when requests were turned away.",
  },
  "turned-away": {
    label: "Turned away",
    what: "Requests a second refused here because every slot and queue place was taken, or a rate limit applied.",
    example: "With a queue limit of 0, a request is refused the moment every slot is busy.",
    hint: "Shown only when it is not zero: add slots or copies, or lengthen the queue.",
  },
  slowest: {
    label: "Slowest work under",
    what: "The time within which 99 in 100 requests finished their work here, not counting time waiting in line.",
    example: "12 ms waiting in line and 25 ms of work reads 25 ms here: the wait is not in it.",
    hint: "If this is small but callers are slow, look at the queue, not the work.",
  },
  queue: {
    label: "Queue limit",
    what: "How many requests may wait for a free slot. Past this limit they are turned away.",
    example: "With a queue limit of 100, the 101st waiting request is turned away.",
    hint: "A long queue hides overload as latency; a short one turns requests away sooner.",
  },
} satisfies Record<string, Help>;

/** Every help id with its words, typed by id so a help button cannot name a missing entry. */
export const HELP = { ...BASE, ...WIRE_HELP, ...RESULTS_HELP } satisfies Record<
  string,
  Help
> as Record<keyof typeof BASE | keyof typeof WIRE_HELP | keyof typeof RESULTS_HELP, Help>;

export type HelpId = keyof typeof HELP;

/** The name a help button gives: "About Time". */
export const HELP_LABEL = Object.fromEntries(
  Object.entries(HELP).map(([id, h]) => [id, h.label]),
) as Record<HelpId, string>;
