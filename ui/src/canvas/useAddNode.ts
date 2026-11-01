// Adding a component from the palette (a click, or the command palette): it lands in free space in
// the visible middle and is selected, so its details and dock open at once. A drop keeps the exact
// spot the person chose, so it does not come through here.

import { useDraft } from "../store/draft";
import { useSelected } from "../store/view";
import { canvasBridge } from "./bridge";
import { freeSpot } from "./viewport";

export function useAddNode(): (kind: string) => void {
  const addNode = useDraft((s) => s.addNode);
  const [, setSelected] = useSelected();
  return (kind) => {
    const centre = canvasBridge.centre();
    const taken = Object.values(useDraft.getState().view?.layout ?? {});
    void addNode(kind, centre && freeSpot(centre, taken)).then((name) => {
      if (name) void setSelected(name);
    });
  };
}
