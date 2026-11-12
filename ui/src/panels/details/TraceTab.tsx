// The Trace tab of every mode, in the rail or popped out into the drawer along the bottom: the
// batch Log, the live Trace, or Admin's request feed over the live Trace. One drawer for all three
// (logDrawer.ts); the rail says where the list went while it is out.

import { LazyBoundary } from "../../kit/LazyBoundary";
import { useMode } from "../../data/store/mode";
import { AdminFeed } from "./AdminFeed";
import { LiveTrace } from "./LiveTrace.lazy";
import { useLogDrawer } from "./logDrawer";
import { LogTab } from "./LogTab.lazy";
import { LogSkeleton } from "./Skeletons";

export function TraceTab({ drawer = false }: { drawer?: boolean }) {
  const mode = useMode((s) => s.mode);
  const out = useLogDrawer((s) => s.out);
  const fallback = <LogSkeleton />;
  if (mode === "batch") {
    return (
      <LazyBoundary fallback={fallback}>
        <LogTab drawer={drawer} />
      </LazyBoundary>
    );
  }
  // Admin: the feed (with Edit) over the live Trace, starting on the real requests. While the
  // trace is out only its note stays in the rail, so the feed is not polled twice.
  const feed = mode === "admin" && !(out && !drawer);
  return (
    <>
      {feed ? (
        <>
          <AdminFeed />
          <div className="border-t border-border" />
        </>
      ) : null}
      <LazyBoundary fallback={fallback}>
        <LiveTrace drawer={drawer} initialEndpoint={mode === "admin" ? "real" : ""} />
      </LazyBoundary>
    </>
  );
}
