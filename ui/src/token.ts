// The control token arrives in the URL fragment (`#token=...`, never sent to a server), exactly as
// for the status page: keep it in sessionStorage for this tab, then take it off the address bar.

export const TOKEN_KEY = "kaanal-token";

/** The token of this page load, for when sessionStorage is blocked (the API client reads it first). */
let memory: string | null = null;

export function currentToken(): string | null {
  return memory;
}

/** The slice of `window` this module touches; a test passes a fake. */
export interface TokenEnv {
  location: { hash: string; pathname: string; search: string };
  history: { replaceState(state: unknown, title: string, url: string): void };
  sessionStorage: Pick<Storage, "getItem" | "setItem">;
}

/** Reads `#token=` into sessionStorage and strips the fragment; returns the token, or null. */
export function captureToken(env: TokenEnv): string | null {
  const params = new URLSearchParams(env.location.hash.slice(1));
  const fromHash = params.get("token");
  // a `#token=` with nothing after it is still taken off the address bar
  if (params.has("token")) {
    env.history.replaceState(null, "", env.location.pathname + env.location.search);
  }
  if (fromHash) {
    memory = fromHash;
    try {
      env.sessionStorage.setItem(TOKEN_KEY, fromHash);
    } catch {
      // storage blocked: the token still works for this page load
    }
    return fromHash;
  }
  try {
    return env.sessionStorage.getItem(TOKEN_KEY);
  } catch {
    return null;
  }
}
