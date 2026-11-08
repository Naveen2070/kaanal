import { expect, test, vi } from "vite-plus/test";
import { liveUrl, openLive, type SourceLike } from "./liveClient";
import type { LiveEvent } from "./liveTypes";

/** A fake `EventSource`: the test fires its events by hand. */
class Fake implements SourceLike {
  handlers = new Map<string, (e: MessageEvent) => void>();
  onerror: ((e: Event) => void) | null = null;
  closed = false;
  addEventListener(type: string, fn: (e: MessageEvent) => void) {
    this.handlers.set(type, fn);
  }
  close() {
    this.closed = true;
  }
  emit(type: string, data: unknown) {
    this.handlers.get(type)?.({
      data: typeof data === "string" ? data : JSON.stringify(data),
    } as MessageEvent);
  }
}

test("a_page_load_says_fresh_and_a_reconnect_does_not", () => {
  expect(liveUrl(true, "tok")).toBe("/__kaanal/live?fresh=1&token=tok");
  expect(liveUrl(false, "tok")).toBe("/__kaanal/live?token=tok");
  expect(liveUrl(false, null)).toBe("/__kaanal/live?");
});

test("events_become_typed_live_events", () => {
  const src = new Fake();
  const seen: LiveEvent[] = [];
  openLive({ fresh: true, onEvent: (e) => seen.push(e), onEnd: () => {}, make: () => src });
  src.emit("hello", { session: 3 });
  src.emit("live", { t_ms: 250 });
  src.emit("marker", { at_ms: 1, kind: "pause", text: "paused" });
  src.emit("resync", { session: 3 });
  expect(seen.map((e) => e.type)).toEqual(["hello", "tick", "marker", "resync"]);
});

test("a_message_that_is_not_json_is_ignored_and_the_stream_goes_on", () => {
  const src = new Fake();
  const seen: LiveEvent[] = [];
  openLive({ fresh: false, onEvent: (e) => seen.push(e), onEnd: () => {}, make: () => src });
  src.emit("live", "not json {");
  src.emit("live", { t_ms: 500 });
  expect(seen).toHaveLength(1);
});

test("an_error_closes_the_source_once_and_reports_the_end", () => {
  const src = new Fake();
  const onEnd = vi.fn();
  openLive({ fresh: false, onEvent: () => {}, onEnd, make: () => src });
  src.onerror?.(new Event("error"));
  src.onerror?.(new Event("error"));
  expect(src.closed).toBe(true);
  expect(onEnd).toHaveBeenCalledTimes(1);
});

test("closing_ourselves_does_not_report_an_end_or_deliver_late_events", () => {
  const src = new Fake();
  const onEnd = vi.fn();
  const onEvent = vi.fn();
  const h = openLive({ fresh: false, onEvent, onEnd, make: () => src });
  h.close();
  src.emit("live", { t_ms: 1 });
  src.onerror?.(new Event("error"));
  expect(src.closed).toBe(true);
  expect(onEvent).not.toHaveBeenCalled();
  expect(onEnd).not.toHaveBeenCalled();
});
