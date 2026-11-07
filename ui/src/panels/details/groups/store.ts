// Which Controls groups are open. A store, not component state, so the jump box can open the group
// that holds a setting, and a group the person left open stays open on the next wire and the next
// visit (kept in localStorage, see `settings.ts`). A group nobody has toggled shows its default.

import { create } from "zustand";
import { getGroupsOpen, setGroupsOpen } from "../../../settings";

interface GroupsState {
  open: Record<string, boolean>;
  /** Whether a group is open: what was left, else `fallback` (its default). */
  isOpen: (id: string, fallback: boolean) => boolean;
  set: (id: string, open: boolean) => void;
  toggle: (id: string, fallback: boolean) => void;
}

export const useGroups = create<GroupsState>((set, get) => ({
  open: getGroupsOpen(),
  isOpen: (id, fallback) => get().open[id] ?? fallback,
  set: (id, open) => {
    const next = { ...get().open, [id]: open };
    setGroupsOpen(next);
    set({ open: next });
  },
  toggle: (id, fallback) => get().set(id, !get().isOpen(id, fallback)),
}));

/** Subscribes one group to its own entry. */
export const useGroupOpen = (id: string, fallback: boolean): boolean =>
  useGroups((s) => s.open[id] ?? fallback);
