// Decides, once the draft is loaded, whether the welcome card or the tour is on screen.

import { useEffect } from "react";
import { useDraft } from "../store/draft";
import { projectKeyOf } from "./flags";
import { useFirstRun } from "./store";
import { Tour } from "./Tour";
import { Welcome } from "./Welcome";

export function FirstRun() {
  const projectId = useDraft((s) => s.view?.project_id);
  const phase = useFirstRun((s) => s.phase);
  const begin = useFirstRun((s) => s.begin);
  useEffect(() => {
    // new = a draft with no components yet; read at this moment, not on every edit
    const nodes = useDraft.getState().view?.doc?.nodes ?? {};
    begin(projectKeyOf(projectId), Object.keys(nodes).length === 0);
  }, [begin, projectId]);
  if (phase === "welcome") return <Welcome />;
  if (phase === "tour") return <Tour />;
  return null;
}
