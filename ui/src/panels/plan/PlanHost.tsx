// Mounts the plan dialog while it is open. The plan itself lives in the store, so closing the dialog
// keeps a running plan and its form.

import { usePlan } from "../../data/store/plan";
import { LazyBoundary } from "../../kit/LazyBoundary";
import { PlanDialog } from "./PlanDialog.lazy";

export function PlanHost() {
  const open = usePlan((s) => s.open);
  if (!open) return null;
  return (
    <LazyBoundary fallback={null}>
      <PlanDialog />
    </LazyBoundary>
  );
}
