import { renderHook } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vite-plus/test";

const live = vi.hoisted(() => ({ start: vi.fn(), stop: vi.fn() }));
const loop = vi.hoisted(() => ({ start: vi.fn(), stop: vi.fn() }));
vi.mock("./liveRuntime", () => ({
  startStudioLive: () => {
    live.start();
    return live.stop;
  },
}));
vi.mock("./loopRuntime", () => ({
  startStudioLoop: () => {
    loop.start();
    return loop.stop;
  },
}));

import type { Mode } from "./mode";
import { useModeStream } from "./useModeStream";

beforeEach(() => {
  for (const f of [live.start, live.stop, loop.start, loop.stop]) f.mockClear();
});

test("switching_between_live_and_admin_keeps_the_running_stream", () => {
  const { rerender, unmount } = renderHook(({ m }: { m: Mode }) => useModeStream(m), {
    initialProps: { m: "live" as Mode },
  });
  expect(live.start).toHaveBeenCalledTimes(1);
  rerender({ m: "admin" });
  rerender({ m: "live" });
  // no new session, no lost timeline: the live run was never stopped
  expect(live.start).toHaveBeenCalledTimes(1);
  expect(live.stop).not.toHaveBeenCalled();
  unmount();
  expect(live.stop).toHaveBeenCalledTimes(1);
});

test("batch_stops_the_live_run_and_starts_the_loop_and_back", () => {
  const { rerender } = renderHook(({ m }: { m: Mode }) => useModeStream(m), {
    initialProps: { m: "admin" as Mode },
  });
  rerender({ m: "batch" });
  expect(live.stop).toHaveBeenCalledTimes(1);
  expect(loop.start).toHaveBeenCalledTimes(1);
  rerender({ m: "live" });
  expect(loop.stop).toHaveBeenCalledTimes(1);
  expect(live.start).toHaveBeenCalledTimes(2);
});
