// A new caption in the visible middle of the canvas (from the palette or the command palette).

import { canvasBridge } from "./bridge";
import { addNoteAt } from "./notes";

/** Returns whether a note was added (false at the limit). */
export function addNote(): boolean {
  return addNoteAt(canvasBridge.centre() ?? [0, 0]) !== null;
}
