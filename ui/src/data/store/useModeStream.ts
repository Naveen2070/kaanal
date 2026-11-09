// Which stream runs for a mode: batch listens for a finished run, live and admin share the live
// stream. Live and Admin differ only in what the page shows, so switching between them keeps the
// running session and its timeline; only Batch stops it (a fresh session would start the run over).

import { useEffect } from "react";
import { startStudioLive } from "./liveRuntime";
import { startStudioLoop } from "./loopRuntime";
import type { Mode } from "./mode";

export function useModeStream(mode: Mode): void {
  const batch = mode === "batch";
  useEffect(() => (batch ? startStudioLoop() : startStudioLive()), [batch]);
}
