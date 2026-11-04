import { expect, test } from "vite-plus/test";
import type { LogLine } from "../../data/api/types";
import { callDepths, dur, eventWords, hopDepth, hopType } from "./logFormat";

function line(over: Partial<LogLine> = {}): LogLine {
  return {
    t_ms: 0,
    level: "info",
    request: 1,
    stream: "GET /",
    node: "api",
    replica: 0,
    event: "served",
    wait_ms: 0,
    service_ms: 1,
    status: null,
    note: null,
    ...over,
  };
}

test("every hop kind gets its plain-words badge, neutral", () => {
  const kinds: [string, string][] = [
    ["proc", "Work"],
    ["net", "Network"],
    ["op", "Data"],
    ["cache hit", "Cache hit"],
    ["cache miss", "Cache miss"],
    ["delay", "Delay"],
    ["link request", "Send"],
    ["link response", "Reply"],
    ["node uplink", "Uplink"],
    ["node downlink", "Downlink"],
  ];
  for (const [note, label] of kinds) {
    expect(hopType(line({ note }))).toEqual({ label, tone: "neutral" });
  }
  // no note at all is plain work; a kind from a newer server is shown as written
  expect(hopType(line()).label).toBe("Work");
  expect(hopType(line({ note: "warm up" })).label).toBe("Warm up");
});

test("a failure is badged by its cause in words, and only a failure is bad", () => {
  const fail = (event: LogLine["event"], note: string) =>
    hopType(line({ event, note, wait_ms: null, service_ms: null, level: "error" }));
  expect(fail("limited", "rate limit")).toEqual({ label: "Rate limited", tone: "bad" });
  expect(fail("overflowed", "queue full")).toEqual({ label: "Queue full", tone: "bad" });
  expect(fail("timeout", "deadline passed")).toEqual({ label: "Timed out", tone: "bad" });
  expect(fail("failed", "node down")).toEqual({ label: "Failed", tone: "bad" });
  // a queued hop is still a served hop: neutral badge (the bar carries the wait)
  expect(hopType(line({ event: "queued", wait_ms: 5, note: "proc" })).tone).toBe("neutral");
});

test("call depth is the longest path from a root", () => {
  const d = callDepths([
    { from: "gw", to: "lb" },
    { from: "lb", to: "api" },
    { from: "api", to: "db" },
    // a shortcut does not make db shallower
    { from: "gw", to: "db" },
  ]);
  expect([d.get("gw"), d.get("lb"), d.get("api"), d.get("db")]).toEqual([0, 1, 2, 3]);
});

test("call depth survives a cycle, bare names and a missing graph", () => {
  const d = callDepths([
    { from: "a", to: "b" },
    { from: "b", to: "a" },
    { from: 7, to: true },
  ]);
  expect(d.has("a") && d.has("b")).toBe(true);
  expect(d.get("7")).toBe(0);
  expect(d.get("true")).toBe(1);
  expect(callDepths(undefined).size).toBe(0);
  expect(callDepths(null).size).toBe(0);
  expect(callDepths("nonsense" as never).size).toBe(0);
});

test("a wire hop sits half a step under its caller; an unknown name is depth 0", () => {
  const d = callDepths([{ from: "gw", to: "api" }]);
  expect(hopDepth(line({ node: "api" }), d)).toBe(1);
  expect(hopDepth(line({ from: "api", node: "db" }), d)).toBe(1.5);
  expect(hopDepth(line({ node: "ghost" }), d)).toBe(0);
  expect(hopDepth(line({ node: "api" }), new Map())).toBe(0);
});

test("durations read in the right unit and print zero as zero", () => {
  expect(dur(0)).toBe("0 ms");
  expect(dur(98)).toBe("98 ms");
  expect(dur(9300)).toBe("9.3 s");
  expect(dur(null)).toBe("0 ms");
});

test("a failure reads as a sentence: the status, then the cause when it adds something", () => {
  const f = (status: number | null, note: string | null) =>
    eventWords(
      line({ event: "failed", level: "error", wait_ms: null, service_ms: null, status, note }),
    );
  expect(f(503, "status")).toBe("Failed with 503");
  expect(f(503, "queue full")).toBe("Failed with 503, queue full");
  expect(f(null, "reset")).toBe("Failed, reset");
  expect(f(null, null)).toBe("Failed");
});
