// Whether to show the welcome card, remembered in the browser (spec section 4): one key per project
// and one global "don't show again". Storage can be absent or throw (a private window, blocked site
// data): every access is wrapped and the default is to show.

const SEEN = "kaanal.studio.seen.";
const NEVER = "kaanal.studio.never";

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function write(key: string): void {
  try {
    localStorage.setItem(key, "1");
  } catch {
    // nowhere to remember it: it will show again next time, which is the safe side
  }
}

/** A project is told apart by the server's `project_id`; a draft with no file has none, so the page's origin stands in. */
export function projectKeyOf(projectId: string | undefined): string {
  return projectId && projectId !== "none" ? projectId : location.origin;
}

export function neverAgain(): boolean {
  return read(NEVER) !== null;
}

export function shouldShowWelcome(projectKey: string): boolean {
  return !neverAgain() && read(SEEN + projectKey) === null;
}

export function markSeen(projectKey: string): void {
  write(SEEN + projectKey);
}

export function setNeverAgain(): void {
  write(NEVER);
}
