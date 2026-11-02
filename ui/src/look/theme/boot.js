// Runs before the first paint (a blocking <script> in <head>, same origin, so the CSP allows it): put a
// saved "light" or "dark" on <html> so there is no flash. Plain script, no imports: keep it tiny.
// The same rule lives in theme.ts (initTheme).
try {
  var t = localStorage.getItem("kaanal-theme");
  if (t === "light" || t === "dark") document.documentElement.dataset.theme = t;
} catch {
  // blocked storage: the system setting applies
}
