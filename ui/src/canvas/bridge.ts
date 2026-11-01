// What the canvas lets the rest of the page ask of it without owning its React Flow instance: the
// middle of the visible canvas (where a clicked palette item lands), "fit the view" and the zoom
// buttons. The canvas
// sets these when it mounts; the defaults are for when it is not on screen.

export const canvasBridge: {
  /** The middle of the visible canvas in flow coordinates, or undefined with no canvas. */
  centre: () => [number, number] | undefined;
  fit: () => void;
  zoomIn: () => void;
  zoomOut: () => void;
} = {
  centre: () => undefined,
  fit: () => {},
  zoomIn: () => {},
  zoomOut: () => {},
};
