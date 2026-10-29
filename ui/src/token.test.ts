import { expect, test } from "vite-plus/test";
import { TOKEN_KEY, captureToken, type TokenEnv } from "./token";

function fakeEnv(hash: string) {
  const store = new Map<string, string>();
  const replaced: string[] = [];
  const env: TokenEnv = {
    location: { hash, pathname: "/__kaanal/studio", search: "" },
    history: { replaceState: (_s, _t, url) => void replaced.push(url) },
    sessionStorage: {
      getItem: (k) => store.get(k) ?? null,
      setItem: (k, v) => void store.set(k, v),
    },
  };
  return { env, store, replaced };
}

test("reads_the_token_from_the_fragment_and_clears_it", () => {
  const { env, store, replaced } = fakeEnv("#token=abc123");
  expect(captureToken(env)).toBe("abc123");
  expect(store.get(TOKEN_KEY)).toBe("abc123");
  expect(replaced).toEqual(["/__kaanal/studio"]);
});

test("a_reload_without_a_fragment_keeps_the_stored_token", () => {
  const { env, store } = fakeEnv("");
  store.set(TOKEN_KEY, "kept");
  expect(captureToken(env)).toBe("kept");
});

test("an_empty_token_fragment_is_cleared_and_keeps_the_stored_token", () => {
  const { env, store, replaced } = fakeEnv("#token=");
  store.set(TOKEN_KEY, "kept");
  expect(captureToken(env)).toBe("kept");
  expect(store.get(TOKEN_KEY)).toBe("kept");
  expect(replaced).toEqual(["/__kaanal/studio"]);
});
