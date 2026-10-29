// The Studio app: load the draft, then show the canvas. main.tsx only mounts this.

import { useEffect } from "react";
import { Canvas } from "./canvas/Canvas";
import { useDraft } from "./store/draft";

export function App() {
  const view = useDraft((s) => s.view);
  const error = useDraft((s) => s.error);
  const load = useDraft((s) => s.load);
  useEffect(() => void load(), [load]);
  if (!view) return <p className="p-4">{error ?? "Loading the draft..."}</p>;
  return <Canvas />;
}
