// The lazy boundary for the live Results: a separate chunk, fetched when the tab first opens live.
import { lazy } from "react";

export const LiveResults = lazy(() =>
  import("./LiveResults").then((m) => ({ default: m.LiveResults })),
);
