// Mounts the glossary when it is open. This part is first-load code and tiny; the dialog, its
// words and its tabs are a lazy chunk fetched on the first press of the book button.

import { lazy } from "react";
import { LazyBoundary } from "../../kit/LazyBoundary";
import { useGuide } from "./store";

const Glossary = lazy(() => import("./Glossary").then((m) => ({ default: m.Glossary })));

export function GuideHost() {
  const open = useGuide((s) => s.open);
  if (!open) return null;
  return (
    <LazyBoundary fallback={null}>
      <Glossary />
    </LazyBoundary>
  );
}
