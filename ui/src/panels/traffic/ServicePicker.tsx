// "Which services sit behind the gateway": an All services box and one box per service, shared by the
// traffic builder and the gateway's Controls tab. Picking several means they are called in parallel.
// A service the gateway has no wire to says so; it is connected when the choice is applied.

import type { EdgeDoc } from "../../data/api/types";

export function ServicePicker({
  services,
  gateway,
  edges,
  value,
  onChange,
}: {
  services: string[];
  gateway: string | null;
  edges: EdgeDoc[];
  value: string[];
  onChange: (next: string[]) => void;
}) {
  if (services.length === 0) {
    return (
      <p className="m-0 text-xs text-muted">
        There are no services yet. Add one on the canvas first.
      </p>
    );
  }
  const toggle = (name: string) =>
    onChange(value.includes(name) ? value.filter((t) => t !== name) : [...value, name]);
  return (
    <div className="flex flex-col gap-1.5">
      <label className="flex items-center gap-2 font-medium">
        <input
          type="checkbox"
          className="size-5"
          checked={services.every((s) => value.includes(s))}
          onChange={(e) => onChange(e.currentTarget.checked ? [...services] : [])}
        />
        All services
      </label>
      <div className="flex flex-wrap gap-x-5 gap-y-1.5">
        {services.map((s) => {
          const linked = edges.some((e) => String(e.from) === gateway && String(e.to) === s);
          return (
            <label key={s} className="flex items-center gap-2">
              <input
                type="checkbox"
                className="size-5"
                checked={value.includes(s)}
                onChange={() => toggle(s)}
              />
              {s}
              {linked ? null : <span className="text-xs text-muted">(not connected yet)</span>}
            </label>
          );
        })}
      </div>
      <p className="m-0 text-xs text-muted">
        More than one is called at the same time; the request waits for the slowest.
      </p>
    </div>
  );
}
