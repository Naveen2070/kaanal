// The lazy boundary for the Results tab: a separate chunk, fetched when the tab first opens.
import { lazy } from "react";

export const Results = lazy(() => import("./Results").then((m) => ({ default: m.Results })));
