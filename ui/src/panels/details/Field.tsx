// One labelled setting with a plain-words hint under it. A number is committed on blur or Enter,
// not on every keystroke, so half-typed values never reach the draft.

import { useId, useState } from "react";
import type { HelpId } from "../../overlays/help/copy";
import { HelpPopover } from "../../overlays/help/HelpPopover";
import type { BalanceChoice } from "./fields";
import { display, fromUnit, pickUnit, toUnit, type Unit } from "./units";

/** A field's label with its help button beside it (outside the label, so pressing it never reaches the control). */
export function FieldLabel({
  htmlFor,
  label,
  help,
}: {
  htmlFor: string;
  label: string;
  help?: HelpId;
}) {
  return (
    <div className="flex items-center gap-0.5">
      <label htmlFor={htmlFor} className="text-sm font-medium">
        {label}
      </label>
      {help ? <HelpPopover id={help} label={label} /> : null}
    </div>
  );
}

export function NumberField({
  label,
  help,
  quietHint = false,
  hint,
  value,
  onCommit,
  min = 0,
  max,
  step,
  integer = false,
  placeholder,
  units,
}: {
  label: string;
  /** The jargon behind the label, as a help popover. */
  help?: HelpId;
  /**
   * The help button says what the hint says, so the sentence is kept for screen readers only (it is
   * still what the box is described by). Only with `help`; a hint with a caveat of its own stays.
   */
  quietHint?: boolean;
  hint: string;
  /** In the base unit (the first of `units`, when there are any): what the file holds. */
  value: number | null;
  /** May return whether the edit was accepted; a refusal puts the real value back in the box. */
  onCommit: (n: number) => Promise<boolean> | void;
  /** Limits in the base unit too. */
  min?: number;
  max?: number;
  step?: number | "any";
  integer?: boolean;
  /** Shown in an empty box (a setting with no value, such as "No limit"). */
  placeholder?: string;
  /**
   * Units to pick from, smallest first (see `units.ts`). The box then shows and takes the chosen
   * unit; `value`, `min`, `max` and what `onCommit` gets stay in the base unit.
   */
  units?: readonly Unit[];
}) {
  const id = useId();
  // until a unit is picked, the one that reads best for the value; after, the person's choice
  const [chosen, setChosen] = useState<Unit | null>(null);
  const unit: Unit | null = units ? (chosen ?? pickUnit(units, value)) : null;
  const shown = (base: number) => (unit ? display(base, unit) : base);
  const commit = (input: HTMLInputElement) => {
    const raw = input.value;
    // untouched: the box shows a rounded figure, which must never be written back over the real one
    if (raw === (value === null ? "" : String(shown(value)))) return;
    const typed = Number(raw);
    const n = unit ? fromUnit(typed, unit) : typed;
    const ok =
      raw.trim() !== "" &&
      Number.isFinite(n) &&
      n >= min &&
      (max === undefined || n <= max) &&
      (!integer || Number.isInteger(n));
    const back = () => {
      input.value = value === null ? "" : String(shown(value));
    };
    // out of range or not whole: the draft is never asked, and the box shows what is really set
    if (!ok) return back();
    if (n === value) return;
    void Promise.resolve(onCommit(n)).then((accepted) => accepted === false && back());
  };
  return (
    <div className="mb-2">
      <FieldLabel htmlFor={id} label={label} help={help} />
      <div className="flex items-center gap-1">
        <input
          id={id}
          key={`${value ?? "none"}-${unit?.label ?? ""}`}
          type="number"
          min={unit ? toUnit(min, unit) : min}
          max={max === undefined ? undefined : unit ? toUnit(max, unit) : max}
          step={integer ? 1 : step}
          defaultValue={value === null ? "" : shown(value)}
          placeholder={placeholder}
          aria-describedby={`${id}-hint`}
          onBlur={(e) => commit(e.currentTarget)}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit(e.currentTarget);
          }}
          className="h-7 w-28 rounded border border-control bg-sheet text-text px-1"
        />
        {units && unit ? (
          <select
            id={`${id}-unit`}
            aria-label={`${label} unit`}
            value={unit.label}
            onChange={(e) =>
              setChosen(units.find((u) => u.label === e.currentTarget.value) ?? null)
            }
            className="h-7 rounded border border-control bg-sheet px-1 text-text"
          >
            {units.map((u) => (
              <option key={u.label} value={u.label}>
                {u.label}
              </option>
            ))}
          </select>
        ) : null}
      </div>
      <p id={`${id}-hint`} className={help && quietHint ? "sr-only" : "text-xs text-muted"}>
        {hint}
      </p>
    </div>
  );
}

/**
 * A number box with a unit option box beside it, for a value its owner keeps as text in the base
 * unit (the first of `units`). Typing is in the chosen unit and `onChange` gets the base unit;
 * picking another unit shows the same amount in it. An empty or half-typed box passes as typed.
 */
export function UnitInput({
  id,
  ariaLabel,
  unitLabel,
  value,
  onChange,
  units,
  min,
  max,
  placeholder,
  className = "",
}: {
  id?: string;
  /** Names the number box when there is no visible label for it. */
  ariaLabel?: string;
  /** Names the option box for a screen reader. */
  unitLabel: string;
  value: string;
  onChange: (baseValue: string) => void;
  units: readonly Unit[];
  /** Limits in the base unit. */
  min?: number;
  max?: number;
  placeholder?: string;
  /** Classes for both boxes (their look); the number box adds its own width. */
  className?: string;
}) {
  const uid = useId();
  const [unit, setUnit] = useState<Unit>(units[0] as Unit);
  const [typed, setTyped] = useState<string | null>(null);
  const base = Number(value);
  const expected =
    value.trim() === "" || !Number.isFinite(base) ? value : String(display(base, unit));
  // keep what was typed ("1." on the way to "1.5") while it still means the value
  const shown =
    typed !== null && typed.trim() !== "" && Number(typed) === Number(expected) ? typed : expected;
  return (
    <>
      <input
        id={id ?? uid}
        aria-label={ariaLabel}
        type="number"
        min={min === undefined ? undefined : toUnit(min, unit)}
        max={max === undefined ? undefined : toUnit(max, unit)}
        step="any"
        placeholder={placeholder}
        value={shown}
        onChange={(e) => {
          const t = e.currentTarget.value;
          setTyped(t);
          const n = Number(t);
          onChange(t.trim() === "" || !Number.isFinite(n) ? t : String(fromUnit(n, unit)));
        }}
        className={`${className} w-20`}
      />
      <select
        id={`${id ?? uid}-unit`}
        aria-label={unitLabel}
        value={unit.label}
        onChange={(e) => setUnit(units.find((u) => u.label === e.currentTarget.value) ?? unit)}
        className={className}
      >
        {units.map((u) => (
          <option key={u.label} value={u.label}>
            {u.label}
          </option>
        ))}
      </select>
    </>
  );
}

/** The balance choice: the default, round robin, random, least connections or power of two choices; a weighted one is shown but edited in the file. */
export function BalanceField({
  hint,
  help,
  quietHint = false,
  value,
  defaultLabel,
  label = "Balance",
  dynamic = true,
  overrides = false,
  onChange,
}: {
  hint: string;
  help?: HelpId;
  /** As on `NumberField`: the help button says the hint; the caveat about the choice stays shown. */
  quietHint?: boolean;
  /** The field's name (default "Balance"). */
  label?: string;
  value: BalanceChoice | "weighted" | null;
  /** What no value means here (the callee's own, or round robin). */
  defaultLabel: string;
  /** Offer least connections and power of two choices (default); a balancer's spread cannot use them. */
  dynamic?: boolean;
  /**
   * An explicit round robin is its own entry only where it overrides something (a wire over its
   * callee's setting). Elsewhere it is the default: one entry, and it clears the field.
   */
  overrides?: boolean;
  onChange: (choice: "" | BalanceChoice) => void;
}) {
  const id = useId();
  const dynamicChoice = value === "least-conn" || value === "p2c";
  const caveat = " Runs choose by the real queues; the live mock plays this as round robin.";
  return (
    <div className="mb-2">
      <FieldLabel htmlFor={id} label={label} help={help} />
      <select
        id={id}
        value={!overrides && value === "round-robin" ? "" : (value ?? "")}
        aria-describedby={`${id}-hint`}
        onChange={(e) => onChange(e.currentTarget.value as "" | BalanceChoice)}
        className="h-7 rounded border border-control bg-sheet text-text px-1"
      >
        <option value="">{defaultLabel}</option>
        {overrides ? <option value="round-robin">Round robin</option> : null}
        <option value="random">Random</option>
        {dynamic ? <option value="least-conn">Least connections</option> : null}
        {dynamic ? <option value="p2c">Power of two choices</option> : null}
        {value === "weighted" ? (
          <option value="weighted" disabled>
            Weighted (edit in kaanal.yaml)
          </option>
        ) : null}
      </select>
      <p id={`${id}-hint`} className={help && quietHint ? "sr-only" : "text-xs text-muted"}>
        {hint}
        {!(help && quietHint) && dynamicChoice ? caveat : ""}
      </p>
      {help && quietHint && dynamicChoice ? <p className="text-xs text-muted">{caveat}</p> : null}
    </div>
  );
}

/** How the server works: an event loop (the default), or a thread per request that holds its slot across its calls. */
export function StyleField({
  value,
  onChange,
}: {
  value: "own" | "call";
  onChange: (choice: "own" | "call") => void;
}) {
  const id = useId();
  return (
    <div className="mb-2">
      <label htmlFor={id} className="block text-sm font-medium">
        Server style
      </label>
      <select
        id={id}
        value={value}
        aria-describedby={`${id}-hint`}
        onChange={(e) => onChange(e.currentTarget.value as "own" | "call")}
        className="h-7 rounded border border-control bg-sheet text-text px-1"
      >
        <option value="own">Event loop (default)</option>
        <option value="call">Thread per request</option>
      </select>
      <p id={`${id}-hint`} className="text-xs text-muted">
        A thread per request holds its slot while it waits on what it calls (Tomcat, sync Gunicorn,
        Puma). With no slot limit yet it gets 200 slots and a queue of 100: change them above.
      </p>
    </div>
  );
}

export function Section({
  title,
  help,
  children,
}: {
  title: string;
  /** A help button beside the title (chart titles on Results). */
  help?: HelpId;
  children: React.ReactNode;
}) {
  return (
    <section className="mb-4 border-b pb-3">
      <h3 className="mb-2 flex items-center gap-0.5 text-[13px] font-semibold text-text">
        {title}
        {help ? <HelpPopover id={help} label={title} /> : null}
      </h3>
      {children}
    </section>
  );
}
