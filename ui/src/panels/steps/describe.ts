// What a step row says, and what the Steps group says when it is folded. Pure words, no layout.

import type { Doc } from "../../data/api/types";
import { chainOf, readChain, type Step } from "./model";

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** One line for a step: `call db · read · ×2 · 80%`, `cache c`, `parallel · 2 branches`. */
export function stepLabel(s: Step): string {
  switch (s.kind) {
    case "call": {
      const parts = [`call ${s.node}`];
      if (s.op) parts.push(s.op);
      if (s.repeat !== undefined && s.repeat !== 1) parts.push(`×${s.repeat}`);
      if (s.prob !== undefined && s.prob < 1) parts.push(`${Math.round(s.prob * 1000) / 10}%`);
      return parts.join(" · ");
    }
    case "cache":
      return `cache ${s.node}`;
    case "parallel":
      return `parallel · ${plural(s.branches.length, "branch", "branches")}`;
    case "locked":
      return "A step this panel cannot edit";
  }
}

/** Every step of a list, branches of a `parallel` included. */
export function allSteps(steps: readonly Step[]): Step[] {
  return steps.flatMap((s) =>
    s.kind === "parallel" ? [s, ...allSteps(s.branches.flatMap((b) => b.steps))] : [s],
  );
}

/** The call steps that carry a retry of their own. */
export const retryingCalls = (steps: readonly Step[]): number =>
  allSteps(steps).filter((s) => s.kind === "call" && s.retry !== undefined).length;

/** The route keys of the document, in file order (none when `endpoints:` is a file path or absent). */
export function routeKeys(doc: Doc | null | undefined): string[] {
  const eps = doc?.endpoints;
  return typeof eps === "object" && eps !== null && !Array.isArray(eps) ? Object.keys(eps) : [];
}

/**
 * The folded summary: how many routes have a chain written, and how many steps and own retries
 * they hold. `only` limits it to the routes one component owns (default: every entry).
 */
export function stepsSummary(doc: Doc | null | undefined, only?: readonly string[]): string {
  let routes = 0;
  let steps = 0;
  let retries = 0;
  for (const key of only ?? routeKeys(doc)) {
    const list = readChain(chainOf(doc, key), []);
    if (!list) continue;
    routes++;
    steps += allSteps(list).length;
    retries += retryingCalls(list);
  }
  if (routes === 0) return "default chains";
  const parts = [plural(routes, "route"), plural(steps, "step")];
  if (retries > 0) parts.push(plural(retries, "retry", "retries"));
  return parts.join(" · ");
}

/** Whether any route has a chain of its own written. */
export const anyChain = (doc: Doc | null | undefined, only?: readonly string[]): boolean =>
  (only ?? routeKeys(doc)).some((k) => chainOf(doc, k) !== undefined);

/** The route keys a component owns (`owners` is the validate answer's route to owner map), in key order. */
export const ownedRoutes = (owners: Readonly<Record<string, string>>, node: string): string[] =>
  Object.keys(owners)
    .filter((k) => owners[k] === node)
    .sort();

/** The components a call step of `node` can name: the targets of the edges that start at it. */
export const calleesOf = (doc: Doc | null | undefined, node: string): string[] => [
  ...new Set((doc?.edges ?? []).filter((e) => String(e.from) === node).map((e) => String(e.to))),
];

/** The cache components a `cache:` step can name. */
export const cacheNodes = (doc: Doc | null | undefined): string[] =>
  Object.entries(doc?.nodes ?? {})
    .filter(([, n]) => n.type === "cache")
    .map(([name]) => name);
