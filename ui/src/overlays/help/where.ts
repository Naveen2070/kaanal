// Where a setting lives on screen, for its glossary entry ("Where to find it"): the link back from the
// glossary to the control, the other half of the help button. Only the settings that have a place on a
// wire's or a gateway's Settings tab are listed; an id here must be a wire help id (a test checks).

import type { WIRE_HELP } from "./copyWire";

type Id = keyof typeof WIRE_HELP;

const WIRE = "Settings tab, with a wire selected";
const NODE = "Settings tab, with a component selected";

export const WHERE: Partial<Record<Id, string>> = {
  bandwidth: `${WIRE}: Bandwidth`,
  "network-latency": `${WIRE}: Network latency`,
  "request-size": `${WIRE}: Request size (also on a Steps call)`,
  "response-size": `${WIRE}: Response size (also on a Steps call)`,
  compress: `${WIRE}: Compress the response, in the folded group Compression and streaming`,
  stream: `${WIRE}: Stream the response, in the folded group Compression and streaming`,
  tls: `${WIRE}: Secure connection, in the folded group Connection`,
  keepalive: `${WIRE}: Calls per connection, in the folded group Connection`,
  loss: `${WIRE}: Packet loss, in the folded group Connection`,
  retry: `${WIRE}: Retry a failed call, in the folded group When a call fails`,
  tries: `${WIRE}: Tries, in When a call fails (a Steps call has the same box)`,
  "first-wait": `${WIRE}: First wait, in When a call fails`,
  growth: `${WIRE}: Growth of the wait, in When a call fails`,
  "longest-wait": `${WIRE}: Longest wait, in When a call fails`,
  jitter: `${WIRE}: Randomise the wait, in When a call fails`,
  "retry-writes": `${WIRE}: Retry writes too, in When a call fails`,
  "retry-budget": `${WIRE}: Limit retries with a budget, in When a call fails`,
  abandon: `${WIRE}: When the caller gives up, in When a call fails`,
  breaker: `${WIRE}: the folded group Circuit breaker`,
  detect: `${WIRE} or a load balancer: Notice a downed copy after`,
  "calls-per-request": `${WIRE}: Calls per request, in the folded group How often it is used`,
  "rate-limit": `${NODE}: Request limit, in the folded group Request limit`,
  burst: `${NODE}: Burst, in the folded group Request limit`,
  cores: `${NODE}: Cores, in the folded group CPUs`,
  balance: `${NODE}: Picking a copy, in the folded group Picking a copy`,
  "step-repeat": "Settings tab, with the gateway selected: Steps, a call row, Repeat",
  "step-chance": "Settings tab, with the gateway selected: Steps, a call row, Chance (%)",
  "step-retry": "Settings tab, with the gateway selected: Steps, a call row, Retry on this call",
};
