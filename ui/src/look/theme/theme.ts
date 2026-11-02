// The theme choice. "system" is the default (CSS `light-dark()` follows the OS); a saved "light" or
// "dark" is put on <html> as `data-theme`. boot.js does the same before the first paint, so this
// module only has to agree with it. Storage can be blocked (private window): never throw.

export type Theme = "light" | "dark" | "system";

const KEY = "kaanal-theme";
const root = () => document.documentElement;

const isChoice = (v: unknown): v is "light" | "dark" => v === "light" || v === "dark";

/** What the page follows now: the attribute on <html>, or the system setting. */
export function getTheme(): Theme {
  const t = root().dataset.theme;
  return isChoice(t) ? t : "system";
}

/** Apply the saved choice, if there is one and storage is readable. */
export function initTheme(): void {
  try {
    const saved = localStorage.getItem(KEY);
    if (isChoice(saved)) root().dataset.theme = saved;
  } catch {
    // blocked storage: stay on the system setting
  }
}

export function setTheme(t: Theme): void {
  if (isChoice(t)) root().dataset.theme = t;
  else delete root().dataset.theme;
  try {
    if (isChoice(t)) localStorage.setItem(KEY, t);
    else localStorage.removeItem(KEY);
  } catch {
    // blocked storage: this page follows the choice, the next one will not remember it
  }
}
