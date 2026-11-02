// The lazy boundary for the Live tab: a separate chunk, fetched when the tab first opens.
import { lazy } from "react";

export const Live = lazy(() => import("./Live").then((m) => ({ default: m.Live })));
