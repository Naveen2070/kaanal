// Moving a selected node with the arrow keys (WCAG 2.5.7, a way to place a node without dragging):
// 8 px a press, 40 px with Shift. Pure, so it is tested without the canvas.

const DIR: Record<string, [number, number]> = {
  ArrowLeft: [-1, 0],
  ArrowRight: [1, 0],
  ArrowUp: [0, -1],
  ArrowDown: [0, 1],
};

/** The new position, or null when the key is not an arrow. */
export function nudge(at: [number, number], key: string, shift: boolean): [number, number] | null {
  const d = DIR[key];
  if (!d) return null;
  const step = shift ? 40 : 8;
  return [at[0] + d[0] * step, at[1] + d[1] * step];
}

/** Where an arrow key means "move the node": not in a field, a slider, a list or a dialog. */
export const NOT_FOR_NUDGE =
  "input,textarea,select,[contenteditable=true],[role=slider],[role=combobox],[role=listbox],dialog";
