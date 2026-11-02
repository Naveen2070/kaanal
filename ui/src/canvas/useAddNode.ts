// Adding a component from the palette (a click, or the command palette): it lands in free space in
// the visible middle and is selected, so its details and dock open at once, with its name ready to
// type over. If another component was selected and may call the new one, a toast offers the wire.
// A drop keeps the exact spot the person chose, so it does not come through here.

import { useDraft } from "../data/store/draft";
import { useSelected } from "../data/store/view";
import { canvasBridge } from "./bridge";
import { focusName } from "./focusName";
import { mayCall } from "./rules";
import { freeSlotIn, freeSpot } from "./viewport";

export function useAddNode(): (kind: string) => void {
  const addNode = useDraft((s) => s.addNode);
  const [selected, setSelected] = useSelected();
  return (kind) => {
    const centre = canvasBridge.centre();
    const region = canvasBridge.region();
    const nodes = useDraft.getState().view?.doc?.nodes ?? {};
    const taken = Object.values(useDraft.getState().view?.layout ?? {});
    const from = selected && nodes[selected] ? selected : null;
    // the first free slot in view; with none, the nearest free spot to the middle
    const spot = (region && freeSlotIn(region, taken)) ?? (centre && freeSpot(centre, taken));
    void addNode(kind, spot).then((name) => {
      if (!name) return;
      focusName.pending = name;
      void setSelected(name);
      if (from && mayCall(nodes[from]?.type, kind))
        void import("./offerConnect").then((m) => m.offerConnect(from, name));
    });
  };
}
