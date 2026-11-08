// How far back the live timeline goes (step 8L, RULES 105 k): 5 minutes unless the person chooses
// more, at most 30. It is a setting of this page, not of the file.

import { useDraft } from "../../data/store/draft";
import { MAX_WINDOW_MIN, MIN_WINDOW_MIN, useLive } from "../../data/store/live";
import { Group } from "./groups/Group";
import { DOC_GROUP } from "./groups/ids";

export function LiveWindow() {
  const minutes = useLive((s) => s.windowMin);
  const set = useLive((s) => s.setWindow);
  const loaded = useDraft((s) => s.view?.doc != null);
  if (!loaded) return null;
  return (
    <Group
      id={DOC_GROUP.live}
      title="Live timeline"
      summary={`the last ${minutes} minutes`}
      changed={minutes !== MIN_WINDOW_MIN}
      onReset={() => set(MIN_WINDOW_MIN)}
    >
      <label className="mb-2 block text-sm font-medium" htmlFor="live-window">
        Keep the last (minutes)
      </label>
      <input
        id="live-window"
        name="live-window"
        type="number"
        min={MIN_WINDOW_MIN}
        max={MAX_WINDOW_MIN}
        step={1}
        value={minutes}
        onChange={(e) => set(e.currentTarget.valueAsNumber)}
        className="h-7 w-20 rounded border border-control bg-sheet px-1 text-text"
        aria-describedby="live-window-hint"
      />
      <p id="live-window-hint" className="mt-1 text-xs text-muted">
        {MIN_WINDOW_MIN} to {MAX_WINDOW_MIN}. A longer window keeps more of the run in this page's
        memory; refreshing the page starts the run over.
      </p>
    </Group>
  );
}
