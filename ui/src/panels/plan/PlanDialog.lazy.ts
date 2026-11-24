// The lazy boundary for the plan dialog: a separate chunk, fetched the first time it opens.
import { lazy } from "react";

export const PlanDialog = lazy(() =>
  import("./PlanDialog").then((m) => ({ default: m.PlanDialog })),
);
