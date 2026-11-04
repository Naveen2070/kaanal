// The control bar (spec 4): a flat drawer under the node card, one row per setting the kind has
// (`controlRows`, R1.99): Time (log 0.1 ms to 60 s), Slots (log 1 to 100,000) and Copies (1 to 64),
// each a slider and a typed box. R1.100: a slider commits on pointer or key release, never on every
// move, and while it moves only the capacity line is live; a typed box commits on Enter or blur; a
// refused or out-of-range number puts the real value back and sends nothing. 0 ms is allowed.

import { useRef, useState } from "react";
import type { NodeDoc, Op } from "../data/api/types";
import { Slider } from "../kit/slider";
import { HelpPopover } from "../overlays/help/HelpPopover";
import {
  controlRows,
  durationMs,
  LIMITS,
  setDuration,
  setReplicas,
  setSlots,
  slotsOf,
  type Row,
} from "../panels/details/fields";
import { useDraft } from "../data/store/draft";
import {
  LOG_STEPS,
  msFromSlider,
  sliderFromMs,
  sliderFromSlots,
  slotsFromSlider,
} from "./logSlider";
import { capacityLine, replicasOf } from "./metrics";

type Id = Row["id"];

/** Arrow and page keys step the thumb (Radix does Home and End); +1 or -1, and how many steps. */
const STEP_KEYS: Record<string, [number, number]> = {
  ArrowRight: [1, 1],
  ArrowUp: [1, 1],
  ArrowLeft: [-1, 1],
  ArrowDown: [-1, 1],
  PageUp: [1, 10],
  PageDown: [-1, 10],
};

/** The keys that move a slider: their release commits, a Tab onto it does not. */
const SLIDER_KEYS = new Set([
  "ArrowLeft",
  "ArrowRight",
  "ArrowUp",
  "ArrowDown",
  "PageUp",
  "PageDown",
  "Home",
  "End",
]);

const nf = (n: number) => n.toLocaleString("en-US", { maximumFractionDigits: 3 });
const plural = (n: number, one: string, many: string) => `${nf(n)} ${n === 1 ? one : many}`;
const whole = (n: number, lim: { min: number; max: number }) =>
  Number.isInteger(n) && n >= lim.min && n <= lim.max;

interface Spec {
  label: string;
  unit: string;
  /** Slider position of a value, and the value at a position. */
  pos: (v: number) => number;
  at: (p: number) => number;
  max: number;
  min: number;
  words: (v: number | null) => string;
  ok: (n: number) => boolean;
  write: (n: number) => Op[];
}

export function ControlBar({
  name,
  config,
  onClose,
  onPending,
  dock = false,
}: {
  name: string;
  config: NodeDoc;
  /** Escape: the card closes the bar and puts focus back on its header. */
  onClose?: () => void;
  /** True while a slider moves and until its edit is sent; the card marks its meter "updating". */
  onPending?: (pending: boolean) => void;
  /** In the dock the rows sit side by side; otherwise they stack, as in a drawer. */
  dock?: boolean;
}) {
  const patch = useDraft((s) => s.patch);
  /** The value of the slider being moved, before it is committed. */
  const [moving, setMoving] = useState<{ id: Id; value: number } | null>(null);
  // the same thing, readable at once: a release and a lost pointer capture can arrive back to back,
  // before React has re-rendered, and the second must find nothing left to commit
  const live = useRef<{ id: Id; value: number } | null>(null);
  /**
   * What is typed in a box and not yet committed. The boxes are controlled by this (the real value
   * shows when a row has none), so a commit never rebuilds the element under the cursor.
   */
  const [texts, setTexts] = useState<Partial<Record<Id, { text: string; base: number | null }>>>(
    {},
  );
  /** The box being edited: its text is kept even if the value changes under it. */
  const [editing, setEditing] = useState<Id | null>(null);
  /** Set while Escape closes the bar: the blur that follows must not commit what was typed. */
  const cancelling = useRef(false);

  const real: Record<Id, number | null> = {
    time: durationMs(config.proc),
    slots: slotsOf(config).value,
    copies: replicasOf(config),
  };
  const shown = (id: Id) => (moving?.id === id ? moving.value : real[id]);
  // typed text outlives a change of the real value (reload, undo, file edit) only while it is edited
  const typedText = (id: Id) => {
    const t = texts[id];
    return t !== undefined && (editing === id || t.base === real[id]) ? t.text : undefined;
  };

  const specs: Record<Id, Spec> = {
    time: {
      label: "Time",
      unit: "milliseconds",
      pos: sliderFromMs,
      at: msFromSlider,
      min: 0,
      max: LOG_STEPS,
      words: (v) => (v === null ? "not set" : plural(v, "millisecond", "milliseconds")),
      // 0 is allowed ("never saturates"); anything under a microsecond is not representable
      ok: (n) => (n === 0 || n >= 0.001) && n >= LIMITS.serviceMs.min && n <= LIMITS.serviceMs.max,
      write: (n) => [setDuration(name, config, "proc", n)],
    },
    slots: {
      label: "Slots",
      unit: "at once",
      pos: sliderFromSlots,
      at: slotsFromSlider,
      min: 0,
      max: LOG_STEPS,
      words: (v) => (v === null ? "no slot limit" : `${plural(v, "slot", "slots")} at once`),
      ok: (n) => whole(n, LIMITS.slots),
      write: (n) => setSlots(name, config, n),
    },
    copies: {
      label: "Copies",
      unit: "copies",
      pos: (v) => v,
      at: (p) => p,
      min: LIMITS.instances.min,
      max: LIMITS.instances.max,
      words: (v) => plural(v ?? 1, "copy", "copies"),
      ok: (n) => whole(n, LIMITS.instances),
      write: (n) => [setReplicas(name, config, n)],
    },
  };

  const send = async (ops: Op[]) => {
    onPending?.(true);
    // a refused edit brings no run, so nothing would clear the marker: clear it here
    if (!(await patch(ops))) onPending?.(false);
  };

  const move = (id: Id, p: number) => {
    live.current = { id, value: specs[id].at(p) };
    setMoving(live.current);
    onPending?.(true);
  };
  /**
   * A key step goes to the next value that differs: on the log Slots scale several positions near
   * the low end round to the same whole number, and a press that changes nothing feels broken.
   */
  const step = (id: Id, e: React.KeyboardEvent) => {
    const key = STEP_KEYS[e.key];
    const v = shown(id);
    if (!key || v === null) return;
    e.preventDefault(); // Radix would step by position, not by value
    const s = specs[id];
    const [dir, n] = key;
    let p = Math.min(Math.max(s.pos(v) + dir * n, s.min), s.max);
    while (s.at(p) === v && p > s.min && p < s.max) p += dir;
    if (s.at(p) !== v) move(id, p);
  };
  const commitSlider = (id: Id) => {
    const m = live.current?.id === id ? live.current : null;
    if (m === null) return; // nothing was moving: a Tab onto it, or a second end of one drag
    live.current = null;
    setMoving(null);
    const s = specs[id];
    const was = real[id];
    // back where it started (a log step can round to a neighbour of the real value)
    if (was !== null && s.pos(m.value) === s.pos(was)) {
      onPending?.(false);
      return;
    }
    void send(s.write(m.value));
  };
  const commitTyped = (id: Id) => {
    const raw = typedText(id);
    if (raw === undefined || cancelling.current) return;
    setTexts((t) => ({ ...t, [id]: undefined })); // the box shows the real value again
    const s = specs[id];
    const n = Number(raw);
    // empty, text or out of range: nothing is sent and the real value is what the box shows
    if (raw.trim() === "" || !Number.isFinite(n) || !s.ok(n)) return;
    if (n !== real[id]) void send(s.write(n));
  };

  const time = shown("time");
  const rows = controlRows(String(config.type), config);
  const line = rows.some((r) => r.id === "time")
    ? capacityLine(shown("slots"), time, shown("copies") ?? 1)
    : null;

  return (
    // nodrag/nopan: using the bar must not drag the node or pan the canvas
    // eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions -- keys bubbling from its controls
    <fieldset
      aria-label={`${name} settings`}
      className={`nodrag nopan m-0 flex min-w-0 flex-col gap-2 border-0 p-0 text-xs ${dock ? "" : "rounded-b-xl border-t border-rule bg-sheet px-3 pt-2 pb-3"}`}
      onKeyDown={(e) => {
        // keys stay here: an arrow on a slider must not also move the selected node
        e.stopPropagation();
        if (e.key === "Escape") {
          // Escape cancels: what is typed goes back (the close moves focus, which blurs the box,
          // and a blur commits), and "updating" clears only if this bar was the one holding it
          setTexts({});
          if (live.current !== null) {
            live.current = null;
            setMoving(null);
            onPending?.(false);
          }
          cancelling.current = true;
          onClose?.();
          cancelling.current = false;
        }
      }}
    >
      {line === null ? null : <p className="m-0 text-text tabular-nums">{line}</p>}
      <div className={dock ? "flex flex-wrap items-start gap-x-8 gap-y-3" : "contents"}>
        {rows.map((row) => {
          const s = specs[row.id];
          const v = shown(row.id);
          return (
            <div key={row.id} className={`flex flex-col gap-1 ${dock ? "w-64" : ""}`}>
              <div className="flex items-center justify-between gap-2">
                <span className="flex items-center gap-1 font-medium text-text">
                  {s.label}
                  <HelpPopover id={row.id} label={s.label} />
                </span>
                {row.editable ? (
                  <span className="flex items-center gap-1 text-muted">
                    <input
                      name={`${row.id}-typed`}
                      type="number"
                      aria-label={`${s.label}, typed (${s.unit})`}
                      min={row.id === "time" ? 0 : 1}
                      max={
                        row.id === "time"
                          ? LIMITS.serviceMs.max
                          : row.id === "slots"
                            ? LIMITS.slots.max
                            : LIMITS.instances.max
                      }
                      step={row.id === "time" ? "any" : 1}
                      value={typedText(row.id) ?? String(v ?? "")}
                      onFocus={() => setEditing(row.id)}
                      onChange={(e) => {
                        const text = e.currentTarget.value;
                        setTexts((t) => ({ ...t, [row.id]: { text, base: real[row.id] } }));
                      }}
                      onBlur={() => {
                        commitTyped(row.id);
                        setEditing(null);
                      }}
                      onKeyDown={(e) => e.key === "Enter" && commitTyped(row.id)}
                      className="h-6 w-20 rounded border border-control bg-sheet px-1 text-right text-text tabular-nums"
                    />
                    {row.id === "time" ? "ms" : null}
                  </span>
                ) : (
                  <span className="text-muted">time varies · {row.why}</span>
                )}
              </div>
              {row.editable ? (
                <Slider
                  aria-label={s.label}
                  aria-valuetext={s.words(v)}
                  min={s.min}
                  max={s.max}
                  step={1}
                  value={[v === null ? s.min : s.pos(v)]}
                  onValueChange={([p]) => p !== undefined && move(row.id, p)}
                  onKeyDown={(e) => step(row.id, e)}
                  onPointerUp={() => commitSlider(row.id)}
                  onLostPointerCapture={() => commitSlider(row.id)}
                  onKeyUp={(e) => SLIDER_KEYS.has(e.key) && commitSlider(row.id)}
                />
              ) : null}
              {row.id === "time" && time === 0 ? (
                <p className="m-0 text-warn">0 ms never saturates</p>
              ) : null}
            </div>
          );
        })}
      </div>
    </fieldset>
  );
}
