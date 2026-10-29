// A scripted `fetch` for tests: each call takes the next answer; the requests are kept to look at.

import { vi } from "vite-plus/test";

export interface Sent {
  url: string;
  method: string;
  headers: Record<string, string>;
  body: unknown;
}

export interface Answer {
  status?: number;
  json?: unknown;
  /** Resolves later than the answers after it, to test ordering. */
  delayMs?: number;
}

export function mockFetch(answers: Answer[]): Sent[] {
  const sent: Sent[] = [];
  let i = 0;
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string, init: RequestInit = {}) => {
      const a = answers[Math.min(i++, answers.length - 1)] ?? {};
      sent.push({
        url,
        method: init.method ?? "GET",
        headers: (init.headers ?? {}) as Record<string, string>,
        body: typeof init.body === "string" ? JSON.parse(init.body) : undefined,
      });
      if (a.delayMs) await new Promise((r) => setTimeout(r, a.delayMs));
      return new Response(a.json === undefined ? "" : JSON.stringify(a.json), {
        status: a.status ?? 200,
      });
    }),
  );
  return sent;
}
