// The lazy boundary for the traffic builder: a separate chunk, fetched the first time it opens.
import { lazy } from "react";

export const TrafficBuilder = lazy(() =>
  import("./TrafficBuilder").then((m) => ({ default: m.TrafficBuilder })),
);
