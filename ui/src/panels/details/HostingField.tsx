// The Hosting box at the top of a component's CPUs group (RULES 122): where it runs (a machine you
// run, a managed service, serverless, someone else's). It decides which instance the picker below
// offers and how the cost estimate prices the node. A kind with no choice shows nothing.

import { useId } from "react";
import type { NodeDoc } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { setFlavor } from "./fields";
import {
  CACHE_ENGINES,
  cacheEngineOf,
  engineOf,
  enginesFor,
  flavorOf,
  hasManagedClasses,
  hasMultiAz,
  hostingHint,
  hostingLabel,
  hostingOf,
  hostingsFor,
  isManagedDb,
  setEngine,
  setMultiAz,
  setHosting,
  type HostingId,
} from "./hosting";

const SELECT = "h-7 max-w-full rounded border border-control bg-sheet px-1 text-text";

export function HostingField({ name, config }: { name: string; config: NodeDoc }) {
  const patch = useDraft((s) => s.patch);
  const id = useId();
  const type = String(config.type ?? "");
  const options = hostingsFor(type);
  if (options.length < 2) return null;
  const now = hostingOf(config);
  return (
    <div className="mb-2">
      <label htmlFor={id} className="block text-sm font-medium">
        Hosting
      </label>
      <select
        id={id}
        value={now}
        aria-describedby={`${id}-hint`}
        onChange={(e) => void patch(setHosting(name, config, e.currentTarget.value as HostingId))}
        className={SELECT}
      >
        {options.map((h) => (
          <option key={h} value={h}>
            {hostingLabel(type, h)}
          </option>
        ))}
      </select>
      <p id={`${id}-hint`} className="m-0 mt-1 text-xs text-muted">
        {hostingHint(type, now)}
      </p>
      {isManagedDb(config) && !hasManagedClasses(config) ? (
        <p className="m-0 mt-1 text-xs text-muted">
          The table has no managed class for a {flavorOf(config)} database: it bills per use or
          publishes no class prices. Set cores and memory yourself, or pick Your own instance with a
          cost per hour.
        </p>
      ) : null}
      {type === "cache" && now === "managed" ? (
        <label className="mt-1 flex flex-col gap-0.5 text-sm font-medium">
          Engine
          <select
            name="engine"
            value={cacheEngineOf(config)}
            onChange={(e) => void patch([setFlavor(name, config, e.currentTarget.value)])}
            className={SELECT}
          >
            {CACHE_ENGINES.map((e) => (
              <option key={e.id} value={e.id}>
                {e.label}
              </option>
            ))}
          </select>
          <span className="text-xs font-normal text-muted">
            The price of a class depends on the engine. The engine is the cache's flavor, so it also
            fills the hit ratio and costs you have not written.
          </span>
        </label>
      ) : null}
      {hasMultiAz(config) ? (
        <label className="mt-1 flex items-center gap-1.5 text-sm">
          <input
            type="checkbox"
            name="multi_az"
            checked={config.multi_az === true}
            onChange={(e) => void patch(setMultiAz(name, config, e.currentTarget.checked))}
          />
          Multi-AZ (a standby in a second zone)
        </label>
      ) : null}
      {isManagedDb(config) && hasManagedClasses(config) ? (
        <label className="mt-1 flex flex-col gap-0.5 text-sm font-medium">
          Engine
          <select
            name="engine"
            value={engineOf(config)}
            onChange={(e) => void patch(setEngine(name, config, e.currentTarget.value))}
            className={SELECT}
          >
            {enginesFor(config).map((e) => (
              <option key={e.id} value={e.id}>
                {e.label}
              </option>
            ))}
          </select>
          <span className="text-xs font-normal text-muted">
            The price of a class depends on the engine.
          </span>
        </label>
      ) : null}
    </div>
  );
}
