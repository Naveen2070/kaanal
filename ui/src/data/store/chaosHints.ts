// The hints for chaos that a run did not feel (canvas/chaosHint.ts), read when a run finishes: chaos
// is live on the server, and a new run is what follows any change to it.

import { useEffect, useState } from "react";
import { chaosHiddenHint, type NodeChaos } from "../../canvas/chaosHint";
import { api } from "../api/client";
import { useDraft } from "./draft";
import { isLive, useMode } from "./mode";
import { useRun } from "./run";

/** The hints for the newest finished run; empty while unknown, when nothing is set or when it failed. */
export function useChaosHints(): { node: string; text: string }[] {
  const run = useRun((s) => s.latest);
  const nodes = useDraft((s) => s.view?.doc?.nodes);
  const [chaos, setChaos] = useState<Record<string, NodeChaos>>({});
  const live = isLive(useMode((s) => s.mode));
  const id = run?.id;
  useEffect(() => {
    let live = true;
    api
      .getChaos()
      .then((c) => live && setChaos((c?.effective?.nodes ?? {}) as Record<string, NodeChaos>))
      .catch(() => live && setChaos({}));
    return () => {
      live = false;
    };
  }, [id]);
  const kinds = Object.fromEntries(
    Object.entries(nodes ?? {}).map(([name, n]) => [name, n?.type as string | undefined]),
  );
  // chaos is felt at once in Live: there is no finished run for it to have missed
  return live ? [] : chaosHiddenHint(chaos, run?.summary, kinds);
}
