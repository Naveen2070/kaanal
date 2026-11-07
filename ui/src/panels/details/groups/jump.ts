// The settings a person can jump to from the search box at the top of a wire's Controls: each is the
// exact label the box shows, the group that holds it (opened on the jump) and a few words people
// use for it. A box that only exists while a switch is on (Tries, with retries off) names a
// `fallback`: the switch, so the jump still lands somewhere useful. `EdgeControls.groups.test.tsx`
// opens a fully switched-on wire and fails if any label here is not on the page.

import { GROUP, NODE_GROUP } from "./ids";

export interface JumpTarget {
  /** The label of the field, exactly as the panel shows it. */
  label: string;
  /** The group to open first; none for a setting in the basics. */
  group?: string;
  /** The label of the switch to land on when this box is not on the page. */
  fallback?: string;
  /** Other words people type for it. */
  words?: string;
}

/** The settings of a component's Controls: the labels the boxes carry (a fully switched-on component shows them all). */
export const NODE_TARGETS: readonly JumpTarget[] = [
  { label: "Service time", words: "proc time speed latency work" },
  { label: "Queue limit", words: "queue waiting backlog" },
  { label: "Timeout", words: "deadline give up" },
  { label: "Balance", group: NODE_GROUP.balance, words: "copies replicas round robin pick" },
  { label: "Server style", group: NODE_GROUP.style, words: "thread event loop hold" },
  { label: "Request limit", group: NODE_GROUP.limit, words: "rate limit rps throttle 429" },
  { label: "Burst", group: NODE_GROUP.limit, fallback: "Request limit", words: "rate limit" },
  { label: "Sends up to", group: NODE_GROUP.network, words: "bandwidth card nic upload" },
  { label: "Receives up to", group: NODE_GROUP.network, words: "bandwidth card nic download" },
];

export const WIRE_TARGETS: readonly JumpTarget[] = [
  { label: "Network latency", words: "delay ping round trip" },
  { label: "Bandwidth", words: "speed rate throughput link" },
  { label: "Request size", words: "payload body upload bytes" },
  { label: "Response size", words: "payload body download bytes" },
  { label: "Secure connection", group: GROUP.connection, words: "tls ssl https handshake" },
  { label: "Packet loss (%)", group: GROUP.connection, words: "drop lossy retransmit" },
  { label: "Calls per connection", group: GROUP.connection, words: "keepalive keep-alive reuse" },
  {
    label: "Compress the response (%)",
    group: GROUP.compress,
    words: "gzip brotli compression size",
  },
  { label: "Stream the response", group: GROUP.compress, words: "streaming buffered chunked" },
  { label: "Retry a failed call", group: GROUP.retry, words: "retries again attempt" },
  {
    label: "Tries (the first counts)",
    group: GROUP.retry,
    fallback: "Retry a failed call",
    words: "attempts retry max",
  },
  {
    label: "First wait",
    group: GROUP.retry,
    fallback: "Retry a failed call",
    words: "backoff delay retry",
  },
  {
    label: "Growth of the wait",
    group: GROUP.retry,
    fallback: "Retry a failed call",
    words: "multiplier exponential backoff retry",
  },
  {
    label: "Longest wait",
    group: GROUP.retry,
    fallback: "Retry a failed call",
    words: "cap maximum backoff retry",
  },
  {
    label: "Randomise the wait",
    group: GROUP.retry,
    fallback: "Retry a failed call",
    words: "jitter spread retry",
  },
  {
    label: "Retry writes too",
    group: GROUP.retry,
    fallback: "Retry a failed call",
    words: "idempotent post put retry",
  },
  {
    label: "Limit retries with a budget",
    group: GROUP.retry,
    fallback: "Retry a failed call",
    words: "throttle tokens retry storm",
  },
  { label: "When the caller gives up", group: GROUP.retry, words: "abandon cancel timeout" },
  {
    label: "Stop calling after repeated failures",
    group: GROUP.breaker,
    words: "circuit breaker open trip",
  },
  { label: "Balance", group: GROUP.usage, words: "round robin least connections p2c copies" },
  { label: "Calls per request", group: GROUP.usage, words: "repeat loop fan-out" },
];

/** The targets whose label or words hold every word typed (in any order and case), best first. */
export function matchTargets(targets: readonly JumpTarget[], text: string): JumpTarget[] {
  const words = text.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return [];
  const hits = targets.filter((t) => {
    const hay = `${t.label} ${t.words ?? ""}`.toLowerCase();
    return words.every((w) => hay.includes(w));
  });
  // a label hit beats a synonym hit; otherwise the panel's own order
  const inLabel = (t: JumpTarget) => words.every((w) => t.label.toLowerCase().includes(w));
  return [...hits.filter(inLabel), ...hits.filter((t) => !inLabel(t))].slice(0, 6);
}
