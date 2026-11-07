// The lazy boundary for the step editor: only a selected gateway needs it.
import { lazy } from "react";

export const StepsGroup = lazy(() =>
  import("./StepsGroup").then((m) => ({ default: m.StepsGroup })),
);
