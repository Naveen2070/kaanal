// Mounts the traffic builder while it is open, so every opening starts from the draft as it is now.

import { LazyBoundary } from "../../kit/LazyBoundary";
import { useTrafficBuilder } from "./store";
import { TrafficBuilder } from "./TrafficBuilder.lazy";

export function TrafficHost() {
  const open = useTrafficBuilder((s) => s.open);
  if (!open) return null;
  return (
    <LazyBoundary fallback={null}>
      <TrafficBuilder />
    </LazyBoundary>
  );
}
