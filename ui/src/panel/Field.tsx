// One labelled setting with a plain-words hint under it. A number is committed on blur or Enter,
// not on every keystroke, so half-typed values never reach the draft.

import { useId } from "react";
import type { HelpId } from "../help/copy";
import { HelpPopover } from "../help/HelpPopover";

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
        aria-describedby={`${id}-hint`}
        onBlur={(e) => commit(e.currentTarget)}
        onKeyDown={(e) => {
          if (e.key === "Enter") commit(e.currentTarget);
        }}
        className="w-28 rounded border px-1"
      />
      <p id={`${id}-hint`} className="text-xs text-neutral-500">
        {hint}
      </p>
    </div>
  );
}

export function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="mb-4 border-b pb-3">
      <h3 className="mb-2 text-xs font-semibold uppercase text-neutral-500">{title}</h3>
      {children}
    </section>
  );
}
