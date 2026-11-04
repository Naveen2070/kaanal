// The Log's one calm toolbar: All or Errors, a request search, and the rest of the filters behind
// a "Filters" disclosure that counts what is on and offers Clear. Every filter that was here
// before is still here, and each builds the same query; only the layout changed.

import { ChevronDown, Search } from "lucide-react";
import type { ComponentProps, ReactNode } from "react";

export interface FilterState {
  errorsOnly: boolean;
  setErrorsOnly: (v: boolean) => void;
  reqText: string;
  setReqText: (v: string) => void;
  /** This component (the selected one), on or off; `selected` null disables it. */
  thisNode: boolean;
  setThisNode: (v: boolean) => void;
  selected: string | null;
  endpoint: string;
  setEndpoint: (v: string) => void;
  streams: string[];
  component: string;
  components: string[];
  setComponent: (v: string) => void;
  wire: string;
  wires: { key: string; label: string }[];
  setWire: (v: string) => void;
  replica: string;
  replicaBad: boolean;
  setReplica: (v: string) => void;
}

const CONTROL =
  "h-8 rounded-md border border-control bg-sheet px-2 text-xs focus-visible:outline-2 focus-visible:outline-accent";

/** How many of the Filters panel's filters are on (the segment and the search count themselves). */
export function activeFilters(f: FilterState): number {
  return [
    f.thisNode && f.selected !== null,
    f.endpoint !== "",
    f.component !== "",
    f.wire !== "",
    f.replica !== "",
  ].filter(Boolean).length;
}

export function Toolbar({
  f,
  open,
  setOpen,
  onClear,
  anyOn,
}: {
  f: FilterState;
  open: boolean;
  setOpen: (v: boolean) => void;
  onClear: () => void;
  anyOn: boolean;
}) {
  const n = activeFilters(f);
  return (
    <div className="mb-2">
      <div className="flex flex-wrap items-center gap-2">
        <fieldset
          aria-label="Which hops"
          className="m-0 flex min-w-0 flex-none rounded-md border-0 bg-well p-0.5"
        >
          <Seg on={!f.errorsOnly} onClick={() => f.setErrorsOnly(false)}>
            All
          </Seg>
          <Seg on={f.errorsOnly} onClick={() => f.setErrorsOnly(true)}>
            Errors only
          </Seg>
        </fieldset>
        <label className="relative min-w-24 flex-1">
          <Search
            size={14}
            aria-hidden
            className="pointer-events-none absolute top-2 left-2 text-muted"
          />
          <input
            type="search"
            inputMode="numeric"
            aria-label="Request"
            placeholder="Find r-…"
            value={f.reqText}
            onChange={(e) => f.setReqText(e.currentTarget.value)}
            className={`${CONTROL} w-full pl-7`}
          />
        </label>
        <button
          type="button"
          aria-expanded={open}
          aria-label={n > 0 ? `Filters (${n} on)` : "Filters"}
          onClick={() => setOpen(!open)}
          className={`${CONTROL} flex flex-none items-center gap-1 hover:bg-raised`}
        >
          Filters
          {n > 0 ? (
            <span className="rounded bg-active px-1 text-[11px] leading-4 tabular-nums">{n}</span>
          ) : null}
          <ChevronDown
            size={14}
            aria-hidden
            className={`text-muted transition-transform ${open ? "rotate-180" : ""}`}
          />
        </button>
        {anyOn ? (
          <button
            type="button"
            onClick={onClear}
            className="h-8 flex-none rounded-md px-1.5 text-xs underline hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
          >
            Clear
          </button>
        ) : null}
      </div>
      {open ? (
        <div className="mt-2 flex flex-wrap items-end gap-x-3 gap-y-2 rounded-lg border border-border bg-well p-2 text-xs">
          <Field label="Show">
            <Chip
              on={f.thisNode && f.selected !== null}
              disabled={f.selected === null}
              title={f.selected ?? "Select a component first"}
              onClick={() => f.setThisNode(!f.thisNode)}
            >
              This component
            </Chip>
          </Field>
          {f.streams.length > 3 ? (
            <label className="flex flex-col gap-1">
              Endpoint
              <select
                value={f.endpoint}
                onChange={(e) => f.setEndpoint(e.currentTarget.value)}
                className={`${CONTROL} max-w-44`}
              >
                <option value="">Any</option>
                {f.streams.map((s) => (
                  <option key={s} value={s}>
                    {s}
                  </option>
                ))}
              </select>
            </label>
          ) : f.streams.length > 0 ? (
            <Field label="Endpoint">
              <div className="flex flex-wrap gap-1.5">
                {f.streams.map((s) => (
                  <Chip
                    key={s}
                    on={f.endpoint === s}
                    onClick={() => f.setEndpoint(f.endpoint === s ? "" : s)}
                  >
                    {s}
                  </Chip>
                ))}
              </div>
            </Field>
          ) : null}
          <label className="flex flex-col gap-1">
            Component
            <select
              value={f.component}
              onChange={(e) => f.setComponent(e.currentTarget.value)}
              className={`${CONTROL} max-w-36`}
            >
              <option value="">Any</option>
              {f.components.map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          </label>
          <label className="flex flex-col gap-1">
            Wire
            <select
              value={f.wire}
              disabled={f.wires.length === 0}
              title={
                f.wires.length === 0
                  ? "Only a wire with a bandwidth logs transfers, and none has one"
                  : undefined
              }
              onChange={(e) => f.setWire(e.currentTarget.value)}
              className={`${CONTROL} max-w-44 disabled:opacity-50`}
            >
              <option value="">Any</option>
              {f.wires.map((l) => (
                <option key={l.key} value={l.key}>
                  {l.label}
                </option>
              ))}
            </select>
          </label>
          <label className="flex flex-col gap-1">
            Instance
            <input
              type="number"
              min={0}
              max={65535}
              value={f.replica}
              aria-invalid={f.replicaBad}
              placeholder="Any"
              onChange={(e) => f.setReplica(e.currentTarget.value)}
              className={`${CONTROL} w-20 tabular-nums`}
            />
          </label>
        </div>
      ) : null}
    </div>
  );
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-1">
      <span>{label}</span>
      {children}
    </div>
  );
}

function Seg({
  on,
  children,
  ...rest
}: { on: boolean } & Omit<ComponentProps<"button">, "aria-pressed">) {
  return (
    <button
      type="button"
      aria-pressed={on}
      {...rest}
      className={`h-7 flex-none whitespace-nowrap rounded px-2.5 text-xs focus-visible:outline-2 focus-visible:outline-accent ${
        on ? "bg-sheet font-medium text-text" : "text-muted hover:text-text"
      }`}
    >
      {children}
    </button>
  );
}

function Chip({
  on,
  children,
  ...rest
}: { on: boolean } & Omit<ComponentProps<"button">, "aria-pressed">) {
  return (
    <button
      type="button"
      aria-pressed={on}
      {...rest}
      className={`h-8 flex-none whitespace-nowrap rounded-md border px-2 text-xs disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-accent ${
        on ? "border-accent bg-active font-medium" : "border-control bg-sheet"
      }`}
    >
      {children}
    </button>
  );
}
