// Deleting wires from the canvas or from the rail: one edit (one undo), then a notice that offers
// the undo, because a wire an endpoint relies on makes the draft invalid and the way back should be
// one click away.

import { useDraft } from "../store/draft";
import { notify } from "../toast/toasts";

/** Removes the `edges` entries at these places, highest first so the places stay true. */
export async function removeWires(places: number[]): Promise<void> {
  const at = [...new Set(places)].sort((a, b) => b - a);
  if (at.length === 0) return;
  const ok = await useDraft.getState().patch(at.map((i) => ({ op: "remove", path: ["edges", i] })));
  if (!ok) return;
  notify.info(at.length === 1 ? "Wire deleted." : `${at.length} wires deleted.`, "wire", {
    label: "Undo",
    run: () => void useDraft.getState().undo(),
  });
}
