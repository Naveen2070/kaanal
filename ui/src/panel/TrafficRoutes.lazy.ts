// The lazy boundary for a gateway's traffic routes.
import { lazy } from "react";

export const TrafficRoutes = lazy(() =>
  import("./TrafficRoutes").then((m) => ({ default: m.TrafficRoutes })),
);
