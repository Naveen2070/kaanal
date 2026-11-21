// The lazy boundary for the sweep chart: a separate chunk.
import { lazy } from "react";

export const SweepChart = lazy(() =>
  import("./SweepChart").then((m) => ({ default: m.SweepChart })),
);
