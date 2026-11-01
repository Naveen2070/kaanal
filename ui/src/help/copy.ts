// The words behind every jargon label (spec 5.2): one plain sentence each, under 25 words. The label
// is what the screen shows; the sentence is what its help popover says.

export const HELP = {
  time: "How long one request holds a slot while it is being worked on. Longer means fewer requests finished each second.",
  slots:
    "How many requests one copy works on at the same moment. More slots, more throughput, until the time per request limits it.",
  copies:
    "Identical copies of this component, each with its own slots and its own queue. Load is shared between them.",
  p99: "99 in every 100 requests finish faster than this. It shows the slow tail that an average hides.",
  goodput:
    "Requests that finished successfully each second, not counting failed or turned-away ones.",
  saturation:
    "The point where a component has no spare capacity. Past it, requests queue up and then get turned away.",
  queue: "How many requests may wait for a free slot. Past this limit they are turned away.",
} as const;

export type HelpId = keyof typeof HELP;

/** The name a help button gives: "About Time". */
export const HELP_LABEL: Record<HelpId, string> = {
  time: "Time",
  slots: "Slots",
  copies: "Copies",
  p99: "p99",
  goodput: "Goodput",
  saturation: "Breaks first",
  queue: "Queue limit",
};
