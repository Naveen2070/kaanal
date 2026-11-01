// The lazy boundary for the charts band: a separate chunk.
import { lazy } from "react";

export const ChartsBand = lazy(() =>
  import("./ChartsBand").then((m) => ({ default: m.ChartsBand })),
);
