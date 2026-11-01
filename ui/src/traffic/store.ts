// Whether the traffic builder is open. The button beside the offered load, the command palette and
// the dialog all go through this, so none of them needs to know the others.

import { create } from "zustand";

export const useTrafficBuilder = create<{ open: boolean }>(() => ({ open: false }));

export const openTrafficBuilder = () => useTrafficBuilder.setState({ open: true });
export const closeTrafficBuilder = () => useTrafficBuilder.setState({ open: false });
