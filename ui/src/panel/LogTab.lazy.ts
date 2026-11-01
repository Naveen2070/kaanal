// The lazy boundary for the Log tab: its code is a separate chunk, fetched when the tab first opens.
import { lazy } from "react";

export const LogTab = lazy(() => import("./LogTab").then((m) => ({ default: m.LogTab })));
