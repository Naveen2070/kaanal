// The live stream and its commands (step 8L). The stream is server-sent events at
// `/__kaanal/live`; `EventSource` cannot set headers, so the token goes in the query (the one
// place the server allows it). `fresh=1` says "this is a page load: start the run over"; an
// automatic reconnect must NOT say it (it would reset the run), so the browser's own reconnect is
// switched off here (an error closes the source and the runtime decides what to do next).

import { readToken } from "../../token";
import type { LiveEvent, LiveHello, Marker, Tick } from "./liveTypes";

/** The slice of `EventSource` this module uses; a test passes a fake. */
export interface SourceLike {
  addEventListener(type: string, fn: (e: MessageEvent) => void): void;
  close(): void;
  onerror: ((e: Event) => void) | null;
}

export interface OpenOptions {
  /** A page load: the server starts a fresh run. */
  fresh: boolean;
  onEvent: (ev: LiveEvent) => void;
  /** The stream ended or failed: the source is closed; reconnect (without `fresh`) if still wanted. */
  onEnd: () => void;
  /** Makes the source; the default is the browser's `EventSource`. */
  make?: (url: string) => SourceLike;
}

export function liveUrl(fresh: boolean, token: string | null): string {
  const q = new URLSearchParams();
  if (fresh) q.set("fresh", "1");
  if (token) q.set("token", token);
  return `/__kaanal/live?${q.toString()}`;
}

/** Opens the stream; the returned `close` ends it without calling `onEnd`. */
export function openLive(o: OpenOptions): { close: () => void } {
  const make = o.make ?? ((url: string) => new EventSource(url));
  const src = make(liveUrl(o.fresh, readToken()));
  let closed = false;
  const on = <T>(type: string, wrap: (data: T) => LiveEvent) =>
    src.addEventListener(type, (e) => {
      if (closed) return;
      try {
        o.onEvent(wrap(JSON.parse(String(e.data)) as T));
      } catch {
        // a message that is not JSON is ignored: the next tick carries on
      }
    });
  on<LiveHello>("hello", (hello) => ({ type: "hello", hello }));
  on<Tick>("live", (tick) => ({ type: "tick", tick }));
  on<Marker>("marker", (marker) => ({ type: "marker", marker }));
  on<LiveHello>("resync", (hello) => ({ type: "resync", hello }));
  src.onerror = () => {
    if (closed) return;
    closed = true;
    src.close();
    o.onEnd();
  };
  return {
    close() {
      closed = true;
      src.close();
    },
  };
}
