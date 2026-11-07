// Quick setups, the header line and the "differs from the default" rows of a component's Controls, the
// way `presets.ts` does them for a wire. A setup is a patch of a few ordinary keys, so one click is one
// undo step and the file shows exactly what it wrote. Every figure is one that is published or that the
// panel already uses: Tomcat's defaults (200 threads, 100 waiting), HikariCP's default pool (10),
// PostgreSQL's default `max_connections` (100); the request limit starts where its Add button does.

import type { NodeDoc, Op } from "../../../data/api/types";
import { callsOthers } from "../../../canvas/rules";
import {
  addRateLimit,
  cardOf,
  holdOf,
  queueOf,
  rateLimitOf,
  setQueue,
  setSlots,
  setStyle,
  slotsOf,
} from "../fields";
import { rateText } from "../units";
import { balanceWord, humanMs, nodeGroupChanged } from "./summaries";
import { detectMsOf } from "../resilience";
import type { DiffRow } from "./retryPreview";

export interface NodeSetup {
  id: string;
  label: string;
  level: "basic" | "advanced";
  /** The chip's tooltip: what it writes and where the number comes from. */
  note: string;
  /** The component already is this (an untouched one is an event loop). */
  pressed: boolean;
  /** What a click writes; empty when there is nothing to change. */
  ops: Op[];
}

const POOL_SMALL = 10;
const POOL_POSTGRES = 100;

/** The setups that fit this component, in the order they are shown. */
export function nodeSetups(name: string, config: NodeDoc): NodeSetup[] {
  const out: NodeSetup[] = [];
  const slots = slotsOf(config);
  const queue = queueOf(config);
  if (callsOthers(config.type)) {
    out.push(
      {
        id: "event-loop",
        label: "Event loop",
        level: "basic",
        note: "A slot is held only while the component works, not while it waits on what it calls (Node, Go, async Python). The default.",
        pressed: holdOf(config) === "own",
        ops: setStyle(name, config, "own"),
      },
      {
        id: "thread-per-request",
        label: "Thread per request",
        level: "basic",
        note: "A slot is held until everything the component calls has answered. With no limit set it gets Tomcat's defaults: 200 threads, 100 waiting.",
        pressed: holdOf(config) === "call",
        ops: setStyle(name, config, "call"),
      },
    );
  }
  if (slots.from === "pool") {
    out.push(
      {
        id: "small-pool",
        label: `Small pool (${POOL_SMALL})`,
        level: "basic",
        note: "A pool of 10 connections: HikariCP's default maximumPoolSize.",
        pressed: slots.value === POOL_SMALL,
        ops: slots.value === POOL_SMALL ? [] : setSlots(name, config, POOL_SMALL),
      },
      {
        id: "postgres-pool",
        label: `Postgres default (${POOL_POSTGRES})`,
        level: "basic",
        note: "100 connections: PostgreSQL's default max_connections.",
        pressed: slots.value === POOL_POSTGRES,
        ops: slots.value === POOL_POSTGRES ? [] : setSlots(name, config, POOL_POSTGRES),
      },
    );
  }
  if (queue !== null) {
    out.push({
      id: "fail-fast",
      label: "Fail fast",
      level: "advanced",
      note: "Queue limit 0: a request is turned away the moment every slot is busy, and nobody waits.",
      pressed: queue === 0,
      ops: queue === 0 ? [] : setQueue(name, config, 0),
    });
  }
  out.push({
    id: "request-limit",
    label: "Limit requests",
    level: "advanced",
    note:
      config.type === "storage"
        ? "Adds a limit of 5,500 requests a second, the figure S3 publishes for reads on one prefix. Change it to your store's."
        : "Adds a limit of 1,000 requests a second: a placeholder, not a published figure. Change it to yours.",
    pressed: rateLimitOf(config) !== null,
    ops: addRateLimit(name, config),
  });
  return out;
}

const PANEL_ORDER = [
  ["balance", "Picking a copy"],
  ["style", "Server style"],
  ["limit", "Request limit"],
] as const;

/** Whether any group of the component holds a setting of the person's. */
export const nodeAnyChanged = (config: NodeDoc): boolean =>
  PANEL_ORDER.some(([g]) => nodeGroupChanged(g, config)) || "bandwidth" in config;

/** "Changed from the defaults: Server style, Request limit", the groups that hold a setting, in the panel's order. */
/** `more` names groups that live outside the component's own config, such as the gateway's Steps. */
export function nodeChangedLine(config: NodeDoc, more: readonly string[] = []): string {
  const titles: string[] = PANEL_ORDER.filter(([g]) => nodeGroupChanged(g, config)).map(
    ([, t]) => t,
  );
  if ("bandwidth" in config) titles.push("Network card");
  titles.push(...more);
  return titles.length
    ? `Changed from the defaults: ${titles.join(", ")}`
    : "Nothing changed from the defaults.";
}

const count = (n: number) => n.toLocaleString("en-US");

/** The settings of one group that differ from the default, old next to new; nothing when none do. */
export function nodeGroupDiff(
  group: "balance" | "style" | "limit" | "network",
  c: NodeDoc,
): DiffRow[] {
  const rows: DiffRow[] = [];
  const add = (label: string, was: string, now: string) => {
    if (was !== now) rows.push({ label, was, now });
  };
  if (group === "style") {
    add("Server style", "Event loop", holdOf(c) === "call" ? "Thread per request" : "Event loop");
  } else if (group === "limit") {
    const l = rateLimitOf(c);
    if (l) {
      add("Request limit", "none", `${count(l.rps)} / s`);
      add("Burst", "none", count(l.burst));
    } else if ("rate_limit" in c) {
      add("Request limit", "none", "set in kaanal.yaml");
    }
  } else if (group === "balance") {
    if ("balance" in c) add("Picking a copy", "round robin", balanceWord(c.balance));
    if ("spread" in c) add("Spread", "round robin", balanceWord(c.spread));
    const d = detectMsOf(c as Record<string, unknown>);
    if (d !== null) add("Notices a downed copy after", "at once", humanMs(d));
  } else {
    const { send, receive } = cardOf(c);
    if (send !== null) add("Sends up to", "no limit", rateText(send));
    if (receive !== null) add("Receives up to", "no limit", rateText(receive));
  }
  return rows;
}
