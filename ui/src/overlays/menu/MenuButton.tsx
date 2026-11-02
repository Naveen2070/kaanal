// The common menu button at the top right: one button for the app-level things that are not the
// draft (the glossary, the profile, and more later). First-load code and tiny; the panel is a lazy
// chunk fetched on the first press. Escape or a press outside closes it and focus returns here.

import { Menu } from "lucide-react";
import { lazy, useEffect, useRef, useState } from "react";
import { LazyBoundary } from "../../kit/LazyBoundary";

const MenuPanel = lazy(() => import("./MenuPanel").then((m) => ({ default: m.MenuPanel })));

export function MenuButton() {
  const [at, setAt] = useState<{ top: number; right: number } | null>(null);
  const button = useRef<HTMLButtonElement>(null);
  const open = at !== null;

  const close = (refocus: boolean) => {
    setAt(null);
    if (refocus) button.current?.focus();
  };

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      close(true);
    };
    const onDown = (e: PointerEvent | MouseEvent) => {
      const t = e.target as Element;
      if (!t.closest("[data-app-menu]") && !button.current?.contains(t)) close(false);
    };
    document.addEventListener("keydown", onKey, true);
    document.addEventListener("pointerdown", onDown);
    return () => {
      document.removeEventListener("keydown", onKey, true);
      document.removeEventListener("pointerdown", onDown);
    };
  }, [open]);

  return (
    <>
      <button
        ref={button}
        type="button"
        aria-label="Menu"
        title="Menu"
        aria-expanded={open}
        aria-haspopup="dialog"
        onClick={() => {
          if (open) return close(false);
          const r = button.current?.getBoundingClientRect();
          if (r) setAt({ top: r.bottom + 8, right: Math.max(8, window.innerWidth - r.right) });
        }}
        className="flex size-8 items-center justify-center rounded-lg text-muted hover:bg-raised hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
      >
        <Menu size={16} aria-hidden />
      </button>
      {at ? (
        <LazyBoundary fallback={null}>
          <MenuPanel top={at.top} right={at.right} onClose={() => close(false)} />
        </LazyBoundary>
      ) : null}
    </>
  );
}
