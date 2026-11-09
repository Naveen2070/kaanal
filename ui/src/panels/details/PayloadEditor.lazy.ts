// The lazy boundary for the response editor: a separate chunk, fetched when a row's Edit response
// is first pressed in Admin mode.
import { lazy } from "react";

export const PayloadEditor = lazy(() =>
  import("./PayloadEditor").then((m) => ({ default: m.PayloadEditor })),
);
