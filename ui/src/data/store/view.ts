// What is selected and which panel tab is open, kept in the address bar so a reload or a shared
// link comes back to the same place. Names only: the control token never goes here (R1.80).

import { parseAsString, parseAsStringLiteral, useQueryState } from "nuqs";

export const TABS = ["controls", "log", "results"] as const;
export type Tab = (typeof TABS)[number];

/** The selected node name (any text: `#`, `/`, spaces and unicode survive the URL), or null. */
export function useSelected() {
  return useQueryState("node", parseAsString);
}

export function useTab() {
  return useQueryState("tab", parseAsStringLiteral(TABS).withDefault("controls"));
}
