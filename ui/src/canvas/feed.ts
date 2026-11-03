// The stats feed a card reads (spec section 8, "4b seam"): today the figures of a finished run,
// later a live stream. A card knows only this interface, never the run store.

import { createContext, useContext } from "react";
import type { LinkSummary, NodeSummary, PipeSummary } from "../data/api/types";

export interface StatsFeed {
  /** The run figures of the node called `name`, or undefined when there is no run or no such node. */
  node(name: string): NodeSummary | undefined;
  /** The run figures of the wire from one node to another; only a wire with a bandwidth has any. */
  link?(from: string, to: string): LinkSummary | undefined;
  /** Every wire with a bandwidth in the run; empty or absent with no run. */
  links?: LinkSummary[];
  /** Every node network pipe with a rate in the run; empty or absent with no run. */
  pipes?: PipeSummary[];
  /** Every node of the run, in the run's order; empty with no run. */
  nodes: NodeSummary[];
  /** True while the figures are older than the draft (a run is in flight or was blocked). */
  dimmed: boolean;
  /** The highest requests per second reaching any node: what an edge's load is measured against. */
  peakVisits: number;
}

export const EMPTY_FEED: StatsFeed = {
  node: () => undefined,
  nodes: [],
  dimmed: false,
  peakVisits: 0,
};

export const FeedContext = createContext<StatsFeed>(EMPTY_FEED);

export function useFeed(): StatsFeed {
  return useContext(FeedContext);
}
