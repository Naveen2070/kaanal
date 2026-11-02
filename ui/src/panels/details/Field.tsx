// One labelled setting with a plain-words hint under it. A number is committed on blur or Enter,
// not on every keystroke, so half-typed values never reach the draft.

import { useId } from "react";
import type { HelpId } from "../../overlays/help/copy";
import { HelpPopover } from "../../overlays/help/HelpPopover";

export function NumberField({
  label,
  help,
  hint,
  value,
  onCommit,
  min = 0,
  max,
  step,
  integer = false,
  placeholder,
}: {
  label: string;
  /** The jargon behind the label, as a help popover. */
  help?: HelpId;
  hint: string;
  value: number | null;
  /** May return whether the edit was accepted; a refusal puts the real value back in the box. */
  onCommit: (n: number) => Promise<boolean> | void;
  min?: number;
  max?: number;
  step?: number | "any";
  integer?: boolean;
  /** Shown in an empty box (a setting with no value, such as "No limit"). */
  placeholder?: string;
}) {
  const id = useId();
  const commit = (input: HTMLInputElement) => {
    const raw = input.value;
    const n = Number(raw);
    const ok =
      raw.trim() !== "" &&
      Number.isFinite(n) &&
      n >= min &&
      (max === undefined || n <= max) &&
      (!integer || Number.isInteger(n));
    const back = () => {
      input.value = value === null ? "" : String(value);
    };
    // out of range or not whole: the draft is never asked, and the box shows what is really set
    if (!ok) return back();
    if (n === value) return;
    void Promise.resolve(onCommit(n)).then((accepted) => accepted === false && back());
  };
  return (
    <div className="mb-2">
      <div className="flex items-center gap-0.5">
        <label htmlFor={id} className="text-sm font-medium">
          {label}
        </label>
        {help ? <HelpPopover id={help} label={label} /> : null}
      </div>
      <input
        id={id}
        key={value ?? "none"}
        type="number"
        min={min}
        max={max}
        step={integer ? 1 : step}
        defaultValue={value ?? ""}
        placeholder={placeholder}
        aria-describedby={`${id}-hint`}
        onBlur={(e) => commit(e.currentTarget)}
        onKeyDown={(e) => {
          if (e.key === "Enter") commit(e.currentTarget);
        }}
        className="h-7 w-28 rounded border border-control bg-sheet text-text px-1"
      />
      <p id={`${id}-hint`} className="text-xs text-muted">
        {hint}
      </p>
    </div>
  );
}

/** The balance choice: the default, round robin or random; a weighted one is shown but edited in the file. */
export function BalanceField({
  hint,
  value,
  defaultLabel,
  onChange,
}: {
  hint: string;
  value: "round-robin" | "random" | "weighted" | null;
  /** What no value means here (the callee's own, or round robin). */
  defaultLabel: string;
  onChange: (choice: "" | "round-robin" | "random") => void;
}) {
  const id = useId();
  return (
    <div className="mb-2">
      <label htmlFor={id} className="block text-sm font-medium">
        Balance
      </label>
      <select
        id={id}
        value={value ?? ""}
        aria-describedby={`${id}-hint`}
        onChange={(e) => onChange(e.currentTarget.value as "" | "round-robin" | "random")}
        className="h-7 rounded border border-control bg-sheet text-text px-1"
      >
        <option value="">{defaultLabel}</option>
        <option value="round-robin">Round robin</option>
        <option value="random">Random</option>
        {value === "weighted" ? (
          <option value="weighted" disabled>
            Weighted (edit in kaanal.yaml)
          </option>
        ) : null}
      </select>
      <p id={`${id}-hint`} className="text-xs text-muted">
        {hint}
      </p>
    </div>
  );
}

export function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="mb-4 border-b pb-3">
      <h3 className="mb-2 text-xs font-semibold uppercase text-muted">{title}</h3>
      {children}
    </section>
  );
}
