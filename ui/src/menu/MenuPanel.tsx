// The common menu's panel: a list of items, each either an action (it runs and the menu closes) or
// a view (it opens in place, with Back). To add a menu later, add one entry to MENU_ITEMS.
// A lazy chunk, with the profile view.

import { ArrowLeft, BookOpen, ChevronRight, CircleUser, type LucideIcon } from "lucide-react";
import { useEffect, useRef, useState, type ComponentType } from "react";
import { useGuide } from "../guide/store";
import { ProfileView } from "./ProfileView";

export interface MenuItem {
  id: string;
  label: string;
  hint: string;
  icon: LucideIcon;
  /** An action: run it and close the menu. */
  run?: () => void;
  /** A view: shown in place of the list, under its own title, with Back. */
  view?: ComponentType;
}

export const MENU_ITEMS: MenuItem[] = [
  {
    id: "glossary",
    label: "Glossary",
    hint: "What every part and figure means",
    icon: BookOpen,
    run: () => useGuide.getState().show(),
  },
  {
    id: "profile",
    label: "Profile",
    hint: "Project, server and the control token",
    icon: CircleUser,
    view: ProfileView,
  },
];

export function MenuPanel({
  top,
  right,
  onClose,
  items = MENU_ITEMS,
}: {
  top: number;
  right: number;
  onClose: () => void;
  items?: MenuItem[];
}) {
  const [open, setOpen] = useState<string | null>(null);
  const list = useRef<HTMLUListElement>(null);
  const current = items.find((i) => i.id === open);
  const View = current?.view;

  // keyboard users land on the first item, and back on the list after a view
  useEffect(() => {
    if (!open) list.current?.querySelector("button")?.focus();
  }, [open]);

  return (
    <dialog
      open
      data-app-menu
      aria-label="Menu"
      style={{ top, right }}
      className="fixed left-auto z-50 m-0 w-80 rounded-2xl border border-border bg-sheet p-2 text-text shadow-float"
    >
      {View && current ? (
        <div className="p-2">
          <button
            type="button"
            onClick={() => setOpen(null)}
            className="mb-2 flex h-7 items-center gap-1.5 rounded-md pr-2 text-xs text-muted hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
          >
            <ArrowLeft size={14} aria-hidden />
            Menu
          </button>
          <h2 className="m-0 text-sm font-semibold">{current.label}</h2>
          <View />
        </div>
      ) : (
        <ul ref={list} className="m-0 list-none p-0">
          {items.map(({ id, label, hint, icon: Icon, run, view }) => (
            <li key={id}>
              <button
                type="button"
                onClick={() => {
                  if (view) setOpen(id);
                  else {
                    run?.();
                    onClose();
                  }
                }}
                className="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
              >
                <Icon size={16} aria-hidden className="shrink-0 text-muted" />
                <span className="min-w-0 flex-1">
                  <span className="block text-sm font-medium">{label}</span>
                  <span className="block text-xs text-muted">{hint}</span>
                </span>
                {view ? <ChevronRight size={14} aria-hidden className="text-muted" /> : null}
              </button>
            </li>
          ))}
        </ul>
      )}
    </dialog>
  );
}
