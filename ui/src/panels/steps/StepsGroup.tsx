// The Steps group (Part 9): a route's chain as a list of rows, one list per route. A step is one line
// ("call db · read · ×2 · 80%"); every edit is one patch of the draft (one undo step). The group is on
// the gateway's Settings, beside the traffic routes, because that is where a route's chain is edited
// today and the Studio does not know which component owns a route.

import { useState } from "react";
import type { Doc } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { Group } from "../details/groups/Group";
import { NODE_GROUP } from "../details/groups/ids";
import { AddPick } from "./AddStep";
import { anyChain, retryingCalls, routeKeys, stepsSummary } from "./describe";
import { addStepOps, chainOf, readChain } from "./model";
import { Announce } from "./keys";
import { StepList, type Pickable } from "./StepRow";

export function StepsGroup({
  doc,
  callable,
  caches = [],
}: {
  doc: Doc;
  callable: string[];
  caches?: string[];
}) {
  const keys = routeKeys(doc);
  const [picked, setPicked] = useState<string | undefined>(undefined);
  const [said, setSaid] = useState("");
  const key = picked !== undefined && keys.includes(picked) ? picked : keys[0];
  if (key === undefined) return null;
  return (
    <Group id={NODE_GROUP.steps} title="Steps" summary={stepsSummary(doc)} changed={anyChain(doc)}>
      <p className="mb-2 text-xs text-muted">
        What a request does at each route, in order. A route with no steps runs the default chain.
      </p>
      <label className="mb-2 flex items-center gap-2 text-xs">
        <span className="text-muted">Route</span>
        <select
          value={key}
          onChange={(e) => setPicked(e.target.value)}
          className="min-w-0 flex-1 rounded-md border border-control bg-sheet px-1.5 py-1 font-mono"
        >
          {keys.map((k) => (
            <option key={k} value={k}>
              {k}
            </option>
          ))}
        </select>
      </label>
      <Announce.Provider value={setSaid}>
        <Route doc={doc} routeKey={key} pick={{ callable, caches }} />
      </Announce.Provider>
      <output className="sr-only">{said}</output>
    </Group>
  );
}

function Route({ doc, routeKey, pick }: { doc: Doc; routeKey: string; pick: Pickable }) {
  const patch = useDraft((s) => s.patch);
  const raw = chainOf(doc, routeKey);
  const steps = readChain(raw, ["endpoints", routeKey, "chain"]);
  if (raw !== undefined && steps === null) {
    return (
      <p className="m-0 text-xs text-muted">
        This chain is written in a form this panel does not read, so it is left as it is.
      </p>
    );
  }
  const add = (step: unknown) => void patch(addStepOps(doc, routeKey, step));
  return (
    <div>
      {steps === null ? (
        <p className="mb-2 text-xs text-muted">
          No steps written for this route: the default chain runs.
        </p>
      ) : (
        <>
          <StepList steps={steps} label={`Steps of ${routeKey}`} pick={pick} />
          {retryingCalls(steps) > 0 ? (
            <p className="mb-2 text-[11px] text-muted">
              A step with its own retry uses it instead of the wire&apos;s.
            </p>
          ) : null}
        </>
      )}
      <AddPick
        options={pick.callable}
        selectLabel="Component to call"
        button="Add call"
        className="text-xs"
        first
        onAdd={add}
      />
      <AddPick
        options={pick.caches}
        selectLabel="Cache to look up"
        button="Add cache lookup"
        className="mt-1.5 text-xs"
        onAdd={(node) => add({ cache: node })}
      />
      <AddPick
        options={pick.callable}
        selectLabel="Component for a parallel block"
        button="Add parallel block"
        className="mt-1.5 text-xs"
        onAdd={(node) => add({ parallel: [node] })}
      />
      {steps !== null ? (
        <button
          type="button"
          onClick={() => void patch([{ op: "remove", path: ["endpoints", routeKey, "chain"] }])}
          className="mt-2 rounded-md border border-control px-2 py-0.5 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
        >
          Use the default chain
        </button>
      ) : null}
    </div>
  );
}
