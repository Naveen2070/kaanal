import { describe, expect, test } from "vite-plus/test";
import { applyStartMode, isLive, useMode } from "./mode";

test("admin_shares_the_live_view_and_batch_does_not", () => {
  expect(isLive("live")).toBe(true);
  expect(isLive("admin")).toBe(true);
  expect(isLive("batch")).toBe(false);
});

test("studio_opens_live_and_the_mode_can_be_set_to_admin", () => {
  expect(useMode.getState().mode).toBe("live");
  useMode.getState().set("admin");
  expect(useMode.getState().mode).toBe("admin");
  useMode.getState().set("live");
});

describe("applyStartMode", () => {
  const env = (search: string) => {
    const calls: string[] = [];
    return {
      calls,
      env: {
        location: { pathname: "/__kaanal/studio", search },
        history: { replaceState: (_s: unknown, _t: string, url: string) => calls.push(url) },
      },
    };
  };

  test("?mode=admin sets Admin once and leaves the address bar clean", () => {
    useMode.setState({ mode: "live" });
    const { env: e, calls } = env("?mode=admin");
    expect(applyStartMode(e)).toBe("admin");
    expect(useMode.getState().mode).toBe("admin");
    expect(calls).toEqual(["/__kaanal/studio"]);
  });

  test("the other parameters stay, and batch and live are taken too", () => {
    const { env: e, calls } = env("?tab=results&mode=batch&node=db");
    expect(applyStartMode(e)).toBe("batch");
    expect(calls).toEqual(["/__kaanal/studio?tab=results&node=db"]);
    expect(applyStartMode(env("?mode=live").env)).toBe("live");
  });

  test("an unknown mode changes nothing and is still taken off the address", () => {
    useMode.setState({ mode: "batch" });
    const { env: e, calls } = env("?mode=root");
    expect(applyStartMode(e)).toBeNull();
    expect(useMode.getState().mode).toBe("batch");
    expect(calls).toEqual(["/__kaanal/studio"]);
  });

  test("no parameter: nothing is touched", () => {
    useMode.setState({ mode: "live" });
    const { env: e, calls } = env("?tab=log");
    expect(applyStartMode(e)).toBeNull();
    expect(calls).toEqual([]);
    expect(useMode.getState().mode).toBe("live");
  });
});
