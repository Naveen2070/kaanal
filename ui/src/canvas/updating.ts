// Which node's settings are mid-edit: a slider is moving, or its edit is on the way and the run it
// starts has not begun. The dock sets it; the card of that node reads it and marks its meter
// "updating". The run feed clears it when the numbers change (a run started or landed).

import { create } from "zustand";

export const useUpdating = create<{ name: string | null }>(() => ({ name: null }));

export const setUpdating = (name: string, on: boolean) =>
  useUpdating.setState((s) => (on ? { name } : s.name === name ? { name: null } : s));
