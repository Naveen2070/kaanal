// The stats feed a card reads (spec section 8, "4b seam"): today the figures of a finished run,
// later a live stream. A card knows only this interface, never the run store.

import { createContext, useContext } from "react";
import type { NodeSummary } from "../api/types";

export interface StatsFeed {
  /** The run figures of the node called `name`, or undefined when there is no run or no such node. */
  node(name: string): NodeSummary | undefined;
  /** True while the figures are older than the draft (a run is in flight or was blocked). */
  dimmed: boolean;
  /** The highest requests per second reaching any node: what an edge's load is measured against. */
  peakVisits: number;
}

export const EMPTY_FEED: StatsFeed = { node: () => undefined, dimmed: false, peakVisits: 0 };

export const FeedContext = createContext<StatsFeed>(EMPTY_FEED);

export function useFeed(): StatsFeed {
  return useContext(FeedContext);
}
