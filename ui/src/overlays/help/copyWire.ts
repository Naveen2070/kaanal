// Help for the wire, retry, breaker and storage settings (steps 5 to 7). One source: the help
// popover says `what`, `example` and `hint`; the glossary chapter "Wire settings" is built from the
// same objects (`more` and `real` add to it), so the two never differ. Every figure in an example is
// worked from the model in docs/web/nodes.md or from a published source named there (RULES 66); a
// default quoted here is the loader's.

import type { Help } from "./copy";

const DEFS = {
  bandwidth: {
    label: "Bandwidth",
    what: "How many bytes this link can carry each second. Calls share it fairly; when it is full they take longer.",
    more: "A transfer lasts its bytes over the narrowest of the sender's card, this wire and the receiver's card. Empty means no limit.",
    example: "5 MB/s carries about 50 replies of 100 KB a second.",
    hint: "Leave it empty unless you are testing a slow or shared link.",
    real: "A network card, a VPN or a cross-region link.",
  },
  "network-latency": {
    label: "Network latency",
    what: "The time one call spends on the wire before the callee starts. It is paid once per call, not each way.",
    more: "The reply is not charged a second latency; its size is charged against the wire's bandwidth. For a round trip, enter the round-trip time.",
    example:
      "A latency of 2 ms on a wire adds 2 ms to every call over it; a route that makes 3 calls over it adds 6 ms.",
    hint: "Enter the delay you measure between the two components. Empty means none, which is right for two components in one process.",
  },
  "request-size": {
    label: "Request size",
    what: "The body each call over this wire carries. Empty passes on the size of the request that reached the caller.",
    example: "A 5 MB upload that a service turns into a 200 KB call to storage: set 200 KB here.",
    hint: "Set it only when this call carries a different size from the one it received.",
  },
  "response-size": {
    label: "Response size",
    what: "The body each call over this wire brings back. Empty passes on the size of the response the caller itself gives.",
    example: "A read that returns 200 KB: set 200 KB, and the link and size costs count it once.",
    hint: "A node's size cost counts the body that moves: a write counts the request, a read the response.",
  },
  "header-bytes": {
    label: "Headers on every call",
    what: "What each call carries besides its body: cookies, tokens and protocol headers. An estimate, not a measurement.",
    more: "The default is 600 B on a request and 400 B on a response. They are charged on every wire transfer, so a call with no body still crosses a wire that has a bandwidth.",
    example:
      "A call with no body still moves 600 + 400 = 1,000 bytes over a wire with a bandwidth.",
    hint: "0 on both turns headers off.",
  },
  compress: {
    label: "Compress the response",
    what: "The share of its size a response keeps on the wire when it is compressed. The component still handles the whole body.",
    more: "Only a response is compressed, and headers are not. There is no universal ratio, so the number is yours.",
    example: "At 30, a 1 MB reply crosses the wire as about 300 KB.",
    hint: "JSON usually compresses to 10 to 30; media that is already compressed stays near 100.",
    real: "gzip or brotli on an HTTP response.",
  },
  stream: {
    label: "Stream the response",
    what: "A streamed body crosses the wire while the other end is still working on it, instead of after it has finished.",
    more: "Buffered, a read costs the store's time and then the wire's, added. Streamed, it costs the first byte plus the size over the slowest stage. A read of a storage node streams by default.",
    example:
      "A 100 MB read from a 100 MB/s store over a 100 MB/s wire: about 2 s buffered, about 1 s streamed.",
    hint: "It only matters on a wire or component with a bandwidth. Leave it on Default unless you are comparing the two.",
    real: "S3, GCS and Azure send a GET as it is read.",
  },
  tls: {
    label: "Secure connection",
    what: "Whether a new connection also does a TLS handshake. After one round trip for TCP, TLS 1.3 adds one and TLS 1.2 adds two.",
    example:
      "At a 40 ms round trip, a new TLS 1.3 connection waits 2 x 40 = 80 ms before its first byte.",
    hint: "Only a new connection pays it (see Calls per connection). A round trip is this wire's latency, so a wire with no latency adds nothing.",
    real: "TCP: RFC 9293. TLS 1.3: RFC 8446. TLS 1.2: RFC 5246.",
  },
  keepalive: {
    label: "Calls per connection",
    what: "How many calls one connection serves before it closes. Only the call that opens a new connection pays for its set-up.",
    more: "A new connection also starts TCP slow start: the first flight is 10 segments and the window doubles every round trip.",
    example: "At 1000 one call in 1000 opens a connection; at 1 every call does.",
    hint: "1 means a new connection for every call; the default is 1000, as nginx's keepalive_requests.",
  },
  loss: {
    label: "Packet loss",
    what: "The share of packets this wire drops. A lost packet costs TCP a recovery: a round trip, or a longer timeout.",
    more: "Small bodies draw their lost packets one by one; a long flow runs at the steady TCP rate of RFC 5348. A lost handshake packet waits 1 s (RFC 6298).",
    example:
      "At 1, one packet in 100 is lost: a 1 MiB body is about 718 segments, so about 7 are lost.",
    hint: "None by default. It adds nothing on a wire with no latency: the cost is in round trips.",
  },
  retry: {
    label: "Retry a failed call",
    what: "A call that times out or gets a 5xx, 429 or reset is tried again after a wait.",
    more: "Every try is a full call: it loads the callee again, which is how retries amplify load.",
    example:
      "With 3 tries and 20% of calls failing, a call makes 1 + 0.2 + 0.04 = 1.24 calls on average.",
    hint: "A 404 or a 400 is never retried. Retries add load, so pair them with a budget or a breaker.",
  },
  tries: {
    label: "Tries",
    what: "Most tries a call makes, from 2 to 5. The first try counts.",
    example: "With 3, a failed call is tried again at most twice.",
    hint: "More tries recover more failures and add more load.",
  },
  "first-wait": {
    label: "First wait",
    what: "The wait before the first retry. Each later wait is longer by the growth.",
    example: "50 ms with a growth of 2 waits 50, 100, then 200 ms, before the random spread.",
    hint: "The default is 50 ms.",
  },
  growth: {
    label: "Growth of the wait",
    what: "Each wait is this many times the one before. 2 doubles it; 1 keeps it the same.",
    example: "From 50 ms, a growth of 2 waits 50, 100, 200; a growth of 1 waits 50, 50, 50.",
    hint: "The default is 2, exponential backoff.",
  },
  "longest-wait": {
    label: "Longest wait",
    what: "A wait never grows past this.",
    example:
      "With 50 ms, a growth of 2 and a limit of 1 s the waits are 50, 100, 200, 400, 800, then 1,000 ms.",
    hint: "The default is 20 s.",
  },
  jitter: {
    label: "Randomise the wait",
    what: "Spreading the waits keeps many callers from retrying in the same instant. Full picks anywhere from 0 to the whole wait.",
    example: "With Full, a 200 ms wait becomes any value from 0 to 200 ms.",
    hint: "Leave it on Full, as the AWS SDK and gRPC do; None makes callers retry in step.",
  },
  "retry-writes": {
    label: "Retry writes too",
    what: "A retried write to a database or service can apply twice, so it is off by default.",
    example: "A write to a storage node is always retried: S3 PUT and DELETE are idempotent.",
    hint: "Turn it on only when the write is safe to repeat.",
  },
  "retry-budget": {
    label: "Limit retries with a budget",
    what: "gRPC's retry throttling: each failed call spends a token, each success gives some back, and retries stop when half the tokens are gone.",
    more: "It is kept per caller copy, as a real client keeps one per process. It is what stops a retry storm from feeding itself.",
    example: "With 10 tokens and no successes, retries stop after 5 failed calls.",
    hint: "Switch it on when retries are on and the callee can be overloaded.",
  },
  "budget-tokens": {
    label: "Budget (tokens)",
    what: "How many tokens the caller starts with, and the most it can hold.",
    example:
      "With 10 tokens, 5 failures in a row with no successes use half of them and stop the retries.",
    hint: "1 to 1000; the default is 10, as gRPC's max_tokens.",
  },
  "budget-refill": {
    label: "Refill per success",
    what: "How much of a token each successful call gives back.",
    example: "At 0.1, ten successful calls buy back one token.",
    hint: "Above 0 and at most 1; the default is 0.1, as gRPC's token_ratio.",
  },
  abandon: {
    label: "When the caller gives up",
    what: "What a callee does with work whose caller has timed out and left. Cancel drops it where it can; continue finishes it.",
    more: "Most servers are not told the caller left, so continue is the realistic setting. With retries on it is the metastable failure of Bronson et al., HotOS 2021.",
    example:
      "A callee at 60% load with a short timeout and 3 tries can tip into failing most requests.",
    hint: "Continue is the realistic setting. A budget or a breaker brings a callee back from a retry storm.",
  },
  breaker: {
    label: "Circuit breaker",
    what: "After enough recent failures the caller stops calling for a while and gets an instant 503 instead.",
    more: "The resilience4j count-based breaker, kept per caller copy and wire. Retry sits outside and the breaker inside: a refused try still waits its backoff and takes no budget token.",
    example:
      "With a window of 50 and a need of 50 calls, 25 failures in the last 50 open it for 10 s.",
    hint: "It then lets a few calls through to see whether the callee is back. Refused calls show as short-circuited.",
  },
  "breaker-window": {
    label: "Calls remembered",
    what: "How many of the latest calls the breaker looks at.",
    example: "At 50, only the last 50 calls count.",
    hint: "1 to 1000; the default is 100.",
  },
  "breaker-min-calls": {
    label: "Calls needed before it can open",
    what: "The fewest recent calls it needs before it may open. Never more than the calls remembered.",
    example: "At 50, one early failure in the first call never opens it.",
    hint: "It counts as the calls remembered when that is larger. The default is 100.",
  },
  "breaker-failure-rate": {
    label: "Open at a failure rate of",
    what: "The share of recent calls that failed at which it opens.",
    example: "At 50, it opens when half or more of the remembered calls failed.",
    hint: "A failure is a 5xx, a timeout or a reset; a 4xx does not count. The default is 50.",
  },
  "breaker-open-for": {
    label: "Stays open for",
    what: "How long every call is refused at once before the breaker tries again.",
    example: "At 10 s, calls get an instant 503 for 10 s.",
    hint: "The default is 60 s.",
  },
  "breaker-trial-calls": {
    label: "Trial calls",
    what: "How many calls it lets through after the wait to test the callee.",
    example: "At 10, ten calls are tried; if fewer than half fail, the breaker closes.",
    hint: "It closes when fewer than the failure rate fail, and opens again otherwise. The default is 10.",
  },
  detect: {
    label: "Notice a downed copy after",
    what: "How long a balancer keeps sending calls to a copy that has gone down, until a health check notices.",
    more: "It is the health-check interval times its unhealthy threshold. A copy that is down from the start of a run is already noticed. An edge's own value wins over the balancer's.",
    example:
      "At 10 s, calls to a copy that goes down at 30 s fail fast until 40 s, then it is skipped.",
    hint: "0, the default, notices at once.",
  },
  "calls-per-request": {
    label: "Calls per request",
    what: "How many times one request calls over this wire.",
    example: "At 3, each request makes 3 calls, so the callee sees 3 times the load.",
    hint: "A chain's own repeat wins over this.",
  },
  "rate-limit": {
    label: "Request limit",
    what: "A token bucket: the requests a second the component accepts. Over it, a request fails with a 429.",
    example: "At 100 a second with a burst of 20, up to 20 may arrive together, then 100 a second.",
    hint: "None by default.",
    real: "An API gateway's throttle.",
  },
  burst: {
    label: "Burst",
    what: "How many requests the bucket holds, so a short spike above the limit is let through.",
    example: "A burst of 20 lets 20 requests arrive together before the limit bites.",
    hint: "At least 1.",
  },
  balance: {
    label: "Balance",
    what: "How traffic entering a component picks one of its copies. It only matters with more than one copy.",
    example:
      "Round robin takes turns; power of two choices compares two random copies and takes the less busy one.",
    hint: "Slow one copy on the Try live tab and compare the p99 of each choice.",
  },
  "first-byte": {
    label: "First byte",
    what: "The time to the first byte of a storage call: the store's own service time.",
    more: "No provider publishes one, so an empty field means 25 ms, a labelled estimate. Write your own measurement.",
    example:
      "A 1 MB read at 80 MB/s with a 25 ms first byte takes 25 + 12.5 = 37.5 ms of store time.",
    hint: "Measure it for your store and region.",
  },
  throughput: {
    label: "Throughput",
    what: "Bytes a second the store moves for one call. A larger body takes longer.",
    example: "At 80 MB/s a 1 MB body adds 1 / 80 s = 12.5 ms.",
    hint: "Request and reply bytes over this rate are added to the first byte. None by default: no size term.",
  },
  "step-repeat": {
    label: "Repeat",
    what: "How many times in a row this step makes its call. Each one is a full call, one after the other.",
    example:
      "A repeat of 3 on a database read makes three reads in a row, so three times the time and the load.",
    hint: "Use it for a loop, such as one lookup per item in a list of ten. Empty uses the wire's own calls per request.",
  },
  "step-chance": {
    label: "Chance",
    what: "The share of requests that make this call at all. The rest skip the step and carry on to the next one.",
    example: "A chance of 30% on a cache fill means 3 requests in 10 make the call.",
    hint: "One draw decides the whole step, repeats included. Empty uses the wire's own chance.",
  },
  "step-retry": {
    label: "This call's retry",
    what: "A retry written on this step. It replaces the wire's retry for this call only.",
    example:
      "The wire retries 3 times; this one step sets 1 try, so only this call gives up at once.",
    hint: "Give a step its own retry when one call needs different patience from the rest over the same wire.",
  },
  "short-circuited": {
    label: "Refused by a breaker",
    what: "A call the caller's breaker refused at once, so it never reached this component.",
    example: "A run where the breaker opened shows these as short-circuited calls on the callee.",
    hint: "They are counted as failures of the call, not as load on the callee.",
  },
} satisfies Record<string, Help>;

/** Typed by id, with every field but `label` and `what` optional (a literal would narrow each entry). */
export const WIRE_HELP: Record<keyof typeof DEFS, Help> = DEFS;
