// Zoom out, the zoom now, zoom in, fit the view and full screen, as one pill at the bottom of the
// screen, next to the details rail. Full screen is the whole page (the browser's own fullscreen), and
// the button says which way it will go.

import { Maximize, Minimize, Minus, Plus, ScanSearch } from "lucide-react";
import { useEffect, useState } from "react";
import { canvasBridge } from "./bridge";
import { useZoom, zoomPercent } from "./zoom";

const button =
  "flex size-8 items-center justify-center rounded-lg text-muted hover:bg-raised hover:text-text focus-visible:outline-2 focus-visible:outline-accent";

export function ZoomPill() {
  const zoom = useZoom((s) => s.zoom);
  const [full, setFull] = useState(() => document.fullscreenElement !== null);
  useEffect(() => {
    const on = () => setFull(document.fullscreenElement !== null);
    document.addEventListener("fullscreenchange", on);
    return () => document.removeEventListener("fullscreenchange", on);
  }, []);
  const toggleFull = () => {
    // a browser may refuse (no gesture, an embedded frame): nothing to do then
    if (document.fullscreenElement) void document.exitFullscreen().catch(() => {});
    else void document.documentElement.requestFullscreen().catch(() => {});
  };
  return (
    <fieldset
      aria-label="View"
      className="pointer-events-auto m-0 flex min-w-0 items-center gap-0.5 rounded-xl border border-border bg-sheet p-1 shadow-float"
    >
      <button
        type="button"
        aria-label="Zoom out"
        title="Zoom out"
        onClick={() => canvasBridge.zoomOut()}
        className={button}
      >
        <Minus size={16} aria-hidden />
      </button>
      <output aria-label="Zoom" className="min-w-12 text-center text-xs font-medium tabular-nums">
        {zoomPercent(zoom)}
      </output>
      <button
        type="button"
        aria-label="Zoom in"
        title="Zoom in"
        onClick={() => canvasBridge.zoomIn()}
        className={button}
      >
        <Plus size={16} aria-hidden />
      </button>
      <span aria-hidden className="mx-0.5 h-5 w-px bg-border" />
      <button
        type="button"
        aria-label="Fit the view"
        title="Fit the view"
        onClick={() => canvasBridge.fit()}
        className={button}
      >
        <ScanSearch size={16} aria-hidden />
      </button>
      <button
        type="button"
        aria-label={full ? "Leave full screen" : "Full screen"}
        title={full ? "Leave full screen" : "Full screen"}
        aria-pressed={full}
        onClick={toggleFull}
        className={button}
      >
        {full ? <Minimize size={16} aria-hidden /> : <Maximize size={16} aria-hidden />}
      </button>
    </fieldset>
  );
}
