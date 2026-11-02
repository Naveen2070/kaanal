// A gateway's traffic routes, on its Controls tab: for each endpoint, which services it forwards to
// (the same picker as the traffic builder). A separate chunk: only a selected gateway needs it.

import type { Doc, EdgeDoc } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { routeOps, serviceNames, targetsOfChain } from "../traffic/model";
import { ServicePicker } from "../traffic/ServicePicker";
import { Section } from "./Field";

export function TrafficRoutes({
  gateway,
  doc,
  edges,
}: {
  gateway: string;
  doc: Doc;
  edges: EdgeDoc[];
}) {
  const patch = useDraft((s) => s.patch);
  const endpoints = (doc.endpoints ?? {}) as Record<string, unknown>;
  return (
    <Section title="Traffic routes">
      <p className="mb-2 text-xs text-muted">
        Which services each endpoint forwards to. Pick several, or all of them: they are called at
        the same time.
      </p>
      {Object.keys(endpoints).map((epKey) => {
        const e = endpoints[epKey];
        const chain = e && typeof e === "object" ? (e as { chain?: unknown }).chain : undefined;
        const targets = targetsOfChain(chain);
        return (
          <fieldset key={epKey} className="m-0 mb-2 min-w-0 border-0 p-0">
            <legend className="font-mono text-xs font-medium text-muted">{epKey}</legend>
            <div className="mt-1">
              {targets === null ? (
                <p className="m-0 text-xs text-muted">
                  A chain written by hand, kept as it is. Change it in the traffic builder.
                </p>
              ) : (
                <ServicePicker
                  services={serviceNames(doc)}
                  gateway={gateway}
                  edges={edges}
                  value={targets}
                  onChange={(next) => void patch(routeOps(doc, epKey, next))}
                />
              )}
            </div>
          </fieldset>
        );
      })}
    </Section>
  );
}
