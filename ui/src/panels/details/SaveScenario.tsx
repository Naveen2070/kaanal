// Save what is set live (chaos and pins) as scenarios/NAME.yaml, so it outlives the run. Saving over
// a name that exists asks first: the server answers 409 and the retry carries `force`.

import { useId, useRef, useState } from "react";
import { ApiError, api } from "../../data/api/client";
import { notify } from "../../shell/toast/toasts";

const btn =
  "min-h-7 rounded border border-control bg-sheet px-2 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-40";

export function SaveScenario({ onSaved }: { onSaved: () => void }) {
  const id = useId();
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [clash, setClash] = useState(false);
  const [error, setError] = useState("");
  const clean = name.trim();
  // a ref, not `busy`: two events in one tick both see the old closure value
  const inFlight = useRef(false);

  const save = async (force: boolean) => {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    setError("");
    try {
      const r = await api.saveScenario(clean, { force });
      setClash(false);
      setName("");
      notify.success(`Saved ${r.name} to scenarios/${r.name}.yaml`, "save-scenario");
      for (const w of r.warnings) notify.warn(w, "save-scenario-warn");
      onSaved();
    } catch (e) {
      // 409 also means "the system changed while this ran, retry": only an existing name asks to replace
      if (e instanceof ApiError && e.status === 409 && /already exists/.test(e.message))
        setClash(true);
      else setError(e instanceof ApiError ? e.message : String(e));
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  };

  return (
    <div className="mt-3 border-t border-control pt-2">
      <p className="mb-1 text-xs text-muted">
        Keep what you set live (chaos and pins) as a scenario in scenarios/.
      </p>
      <div className="flex items-center gap-1 text-xs">
        <input
          id={id}
          aria-label="Scenario name"
          placeholder="name"
          value={name}
          onChange={(e) => {
            setName(e.currentTarget.value);
            setClash(false);
            setError("");
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && clean && !busy) void save(false);
          }}
          className="h-7 min-w-0 flex-1 rounded border border-control bg-sheet px-1 text-text"
        />
        <button
          type="button"
          className={btn}
          disabled={!clean || busy}
          onClick={() => void save(false)}
        >
          Save live
        </button>
      </div>
      {clash ? (
        <div role="alert" className="mt-1 flex items-center gap-1 text-xs">
          <span>{`Replace scenarios/${clean}.yaml?`}</span>
          <button type="button" className={btn} disabled={busy} onClick={() => void save(true)}>
            Replace
          </button>
          <button type="button" className={btn} onClick={() => setClash(false)}>
            Cancel
          </button>
        </div>
      ) : null}
      {error ? (
        <p role="alert" className="mt-1 text-xs text-red-600">
          {error}
        </p>
      ) : null}
    </div>
  );
}
