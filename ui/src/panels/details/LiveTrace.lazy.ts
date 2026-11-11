// The lazy boundary for the live Trace: its code is a separate chunk, fetched when the tab first opens.
import { lazy } from "react";

export const LiveTrace = lazy(() => import("./LiveTrace").then((m) => ({ default: m.LiveTrace })));
