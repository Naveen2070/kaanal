// Decides, once the draft is loaded, whether the welcome card or the tour is on screen.

import { lazy, useEffect } from "react";
import { LazyBoundary } from "../../kit/LazyBoundary";
import { useDraft } from "../../data/store/draft";
import { projectKeyOf } from "./flags";
import { useFirstRun } from "./store";

// the tour is fetched when it first shows, not with the first load
const Welcome = lazy(() => import("./Welcome").then((m) => ({ default: m.Welcome })));
const Tour = lazy(() => import("./Tour").then((m) => ({ default: m.Tour })));

export function FirstRun() {
  const projectId = useDraft((s) => s.view?.project_id);
  const phase = useFirstRun((s) => s.phase);
  const begin = useFirstRun((s) => s.begin);
  useEffect(() => {
    // new = a draft with no components yet; read at this moment, not on every edit
    const nodes = useDraft.getState().view?.doc?.nodes ?? {};
    begin(projectKeyOf(projectId), Object.keys(nodes).length === 0);
  }, [begin, projectId]);
  if (phase === "welcome") {
    return (
      <LazyBoundary key="welcome" fallback={null}>
        <Welcome />
      </LazyBoundary>
    );
  }
  if (phase === "tour") {
    return (
      <LazyBoundary key="tour" fallback={null}>
        <Tour />
      </LazyBoundary>
    );
  }
  return null;
}
