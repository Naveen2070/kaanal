// Small pictures for the glossary, drawn with the real parts: the component icons in their hues,
// the busy meter, the wire's arrowhead and pulse. Their figures are examples, and the picture says
// so. Each is one image to a screen reader (a labelled figure), so the entry's own
// words are what get read. A lazy chunk with the dialog.

import { ArrowRight, X } from "lucide-react";
import { Meter } from "../../canvas/Meter";
import { catColour, kindInfo } from "../../canvas/nodeKinds";
import type { ChainStep, Sample, SlotGroup } from "./content";

const label = (kind: string) => kindInfo(kind)?.label ?? kind;

/** A component's tile: its icon in its hue on a pale tint of it, as on a card and in the palette. */
function Tile({ of, small = false }: { of: string; small?: boolean }) {
  const Icon = kindInfo(of)?.icon;
  const hue = catColour(of);
  return (
    <span
      className={`flex shrink-0 items-center justify-center rounded-lg ${small ? "size-6" : "size-7"}`}
      style={{ background: `color-mix(in srgb, ${hue} 16%, transparent)` }}
    >
      {Icon ? <Icon size={small ? 14 : 16} aria-hidden style={{ color: hue }} /> : null}
    </span>
  );
}

function CardSample({ s }: { s: Extract<Sample, { kind: "card" }> }) {
  const hot = s.ratio >= 0.9 || s.sentence.includes("turning");
  return (
    <div
      className={`w-full max-w-60 overflow-hidden rounded-xl border bg-card ${
        hot ? "border-negative" : "border-border"
      }`}
    >
      <div className="h-[3px]" style={{ background: catColour(s.of) }} />
      <div className="flex flex-col gap-2 px-3 pt-2.5 pb-3">
        <div className="flex items-center gap-2">
          <Tile of={s.of} />
          <span className="truncate text-sm font-semibold">{s.name}</span>
        </div>
        <div className="text-xs text-muted">{s.line}</div>
        <Meter ratio={s.ratio} label={s.sentence} />
        <dl className="m-0 grid grid-cols-[1fr_auto] gap-x-2 gap-y-0.5 text-xs">
          {s.rows.map(([k, v]) => (
            <div key={k} className="contents">
              <dt className="text-muted">{k}</dt>
              <dd className="m-0 text-right font-mono tabular-nums">{v}</dd>
            </div>
          ))}
        </dl>
      </div>
    </div>
  );
}

function Slots({ g, name }: { g: SlotGroup; name?: string }) {
  const places = g.limit ?? g.waiting;
  return (
    <div className="flex flex-col gap-1.5">
      {name ? <span className="text-xs text-muted">{name}</span> : null}
      <div className="flex flex-wrap items-center gap-1">
        {Array.from({ length: g.slots }, (_, i) => (
          <span
            key={i}
            className={`size-4 rounded-sm ${i < g.busy ? "bg-accent" : "border border-control bg-well"}`}
          />
        ))}
        <span className="ml-1 text-xs text-muted">slots</span>
      </div>
      {places > 0 ? (
        <div className="flex flex-wrap items-center gap-1">
          {Array.from({ length: places }, (_, i) => (
            <span
              key={i}
              className={`size-2.5 rounded-full ${i < g.waiting ? "bg-warn-fill" : "border border-control"}`}
            />
          ))}
          <span className="ml-1 text-xs text-muted">
            {g.limit !== undefined ? `waiting (room for ${g.limit})` : "waiting"}
          </span>
        </div>
      ) : null}
    </div>
  );
}

function WireSample({ s }: { s: Extract<Sample, { kind: "wire" }> }) {
  return (
    <div className="flex items-center gap-1">
      <Tile of={s.from} />
      <svg viewBox="0 0 120 32" className="h-8 min-w-0 flex-1" aria-hidden>
        <defs>
          <marker
            id={`arrow-${s.from}-${s.to}`}
            viewBox="0 0 10 10"
            refX="9"
            refY="5"
            markerWidth="10"
            markerHeight="10"
            markerUnits="userSpaceOnUse"
            orient="auto"
          >
            <path d="M1 1 L9 5 L1 9 Z" fill={s.idle ? "var(--muted)" : "var(--lane-0)"} />
          </marker>
        </defs>
        <path
          d="M2 16 H50 V16 H116"
          fill="none"
          stroke={s.idle ? "var(--muted)" : "var(--lane-0)"}
          strokeOpacity={s.idle ? 0.5 : 0.35}
          strokeWidth="2.4"
          strokeDasharray={s.idle ? "3 6" : undefined}
        />
        {s.idle ? (
          <path
            d="M2 16 H116"
            fill="none"
            stroke="none"
            markerEnd={`url(#arrow-${s.from}-${s.to})`}
          />
        ) : (
          <path
            d="M2 16 H116"
            fill="none"
            stroke="var(--lane-0)"
            strokeWidth="3"
            strokeLinecap="round"
            className="kaanal-lane"
            style={{ animationDuration: "1.4s" }}
            markerEnd={`url(#arrow-${s.from}-${s.to})`}
          />
        )}
      </svg>
      <Tile of={s.to} />
    </div>
  );
}

function Figure({ s }: { s: Extract<Sample, { kind: "figure" }> }) {
  return (
    <div className="rounded-lg bg-well px-3 py-2">
      <div className="text-xs text-muted">{s.label}</div>
      <div className="font-mono text-base font-semibold tabular-nums">{s.value}</div>
      {s.slider !== undefined ? (
        <div className="relative mt-2 h-1 rounded-full bg-border">
          <span
            className="absolute top-1/2 size-3 -translate-x-1/2 -translate-y-1/2 rounded-full border border-control bg-sheet"
            style={{ left: `${s.slider * 100}%` }}
          />
        </div>
      ) : null}
    </div>
  );
}

function Draft() {
  const dot = (cls: string) => (
    <span aria-hidden className={`mt-1 size-1.5 shrink-0 rounded-full ${cls}`} />
  );
  return (
    <div className="flex flex-col gap-1.5">
      <span className="flex items-start gap-1.5 text-xs text-warn">
        {dot("bg-warn-fill")}
        Changes not applied · not saved
      </span>
      <span className="flex items-start gap-1.5 text-xs text-muted">
        {dot("bg-ok-fill")}
        Everything saved
      </span>
    </div>
  );
}

function Time({ s }: { s: Extract<Sample, { kind: "time" }> }) {
  const total = s.wait + s.work;
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex h-3 overflow-hidden rounded-full bg-well">
        {s.wait > 0 ? (
          <span className="bg-active" style={{ width: `${(s.wait / total) * 100}%` }} />
        ) : null}
        <span className="bg-accent" style={{ width: `${(s.work / total) * 100}%` }} />
      </div>
      <div className="flex gap-3 text-xs text-muted">
        {s.wait > 0 ? <span>waiting {s.wait} ms</span> : null}
        <span>work {s.work} ms</span>
      </div>
    </div>
  );
}

function Step({ step }: { step: ChainStep }) {
  return (
    <span className={`flex w-14 flex-col items-center gap-1 ${step.skipped ? "opacity-40" : ""}`}>
      <span className="relative">
        <span
          className={`block rounded-lg ${
            step.failed ? "ring-2 ring-negative" : step.skipped ? "ring-1 ring-control" : ""
          }`}
        >
          <Tile of={step.of} />
        </span>
        {step.failed ? (
          <X
            size={12}
            aria-hidden
            className="absolute -top-1.5 -right-1.5 rounded-full bg-negative p-0.5 text-on-primary"
          />
        ) : null}
      </span>
      <span className="text-center text-xs leading-tight text-text">{label(step.of)}</span>
      {step.note ? (
        <span className="text-center text-xs leading-tight text-muted">{step.note}</span>
      ) : null}
    </span>
  );
}

function Chain({ steps }: { steps: ChainStep[] }) {
  return (
    <div className="flex flex-wrap items-start gap-x-0.5 gap-y-2">
      {steps.map((step, i) => (
        <span key={`${step.of}-${i}`} className="flex items-start gap-0.5">
          {i > 0 ? <ArrowRight size={14} aria-hidden className="mt-2 shrink-0 text-muted" /> : null}
          <Step step={step} />
        </span>
      ))}
    </div>
  );
}

function Calls({ rows }: { rows: { from: string; to: string[] }[] }) {
  return (
    <div className="flex flex-col gap-2">
      {rows.map((r) => (
        <div key={r.from} className="flex items-center gap-1.5">
          <Tile of={r.from} small />
          <ArrowRight size={14} aria-hidden className="shrink-0 text-muted" />
          <span className="flex flex-wrap gap-1">
            {r.to.map((k) => (
              <span key={k} title={label(k)}>
                <Tile of={k} small />
              </span>
            ))}
          </span>
        </div>
      ))}
    </div>
  );
}

/** The words a screen reader gets for a sample. */
export function sampleAlt(s: Sample): string {
  switch (s.kind) {
    case "tile":
      return `${label(s.of)} as it looks on the canvas`;
    case "card":
      return `A ${label(s.of).toLowerCase()} card named ${s.name}: ${s.sentence}`;
    case "meters":
      return s.items.map((i) => i.label).join("; ");
    case "slots":
      return s.groups
        .map((g) => `${g.slots} slots, ${g.busy} busy, ${g.waiting} waiting`)
        .join("; ");
    case "wire":
      return `A wire from ${label(s.from)} to ${label(s.to)}${s.idle ? ", dashed: it carried nothing" : ""}`;
    case "figure":
      return `${s.label}: ${s.value}`;
    case "draft":
      return "The draft state: not applied and not saved, then applied and saved";
    case "time":
      return s.wait > 0
        ? `${s.wait} ms waiting, then ${s.work} ms of work`
        : `${s.work} ms of work`;
    case "chain":
      return s.steps.map((x) => `${label(x.of)}${x.note ? ` (${x.note})` : ""}`).join(", then ");
    case "calls":
      return s.rows
        .map((r) => `${label(r.from)} may call ${r.to.map(label).join(", ")}`)
        .join("; ");
  }
}

/** The samples whose figures are made up for the picture; the others show a fact. */
const NUMBERS = new Set<Sample["kind"]>(["card", "meters", "slots", "figure", "time"]);

export function SampleView({ sample }: { sample: Sample }) {
  let body;
  switch (sample.kind) {
    case "tile":
      body = (
        <span className="flex items-center gap-2">
          <Tile of={sample.of} />
          <span className="text-sm font-semibold">{label(sample.of)}</span>
        </span>
      );
      break;
    case "card":
      body = <CardSample s={sample} />;
      break;
    case "meters":
      body = (
        <div className="flex flex-col gap-3">
          {sample.items.map((i) => (
            <div key={i.label}>
              {i.note ? <div className="mb-0.5 text-xs text-muted">{i.note}</div> : null}
              <Meter ratio={i.ratio} label={i.label} />
            </div>
          ))}
        </div>
      );
      break;
    case "slots":
      body = (
        <div className="flex flex-col gap-3">
          {sample.groups.map((g, i) => (
            <Slots key={i} g={g} name={sample.groups.length > 1 ? `copy ${i + 1}` : undefined} />
          ))}
        </div>
      );
      break;
    case "wire":
      body = <WireSample s={sample} />;
      break;
    case "figure":
      body = <Figure s={sample} />;
      break;
    case "draft":
      body = <Draft />;
      break;
    case "time":
      body = <Time s={sample} />;
      break;
    case "chain":
      body = <Chain steps={sample.steps} />;
      break;
    case "calls":
      body = <Calls rows={sample.rows} />;
      break;
  }
  return (
    <figure aria-label={sampleAlt(sample)} className="m-0 flex flex-col gap-1">
      <div aria-hidden>{body}</div>
      {NUMBERS.has(sample.kind) ? (
        <figcaption className="text-xs text-muted">Example figures</figcaption>
      ) : null}
    </figure>
  );
}
