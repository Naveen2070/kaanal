// Under 900 px the canvas is read-only: the nodes as a list, and the reason. Their figures join the
// list with the cards' shared figures (slice 2).

import { NODE_KINDS } from "../canvas/nodeKinds";
import { useDraft } from "../store/draft";

export function ReadonlyNodes() {
  const nodes = Object.entries(useDraft((s) => s.view?.doc?.nodes) ?? {});
  return (
    <section aria-label="Components" className="flex flex-col gap-3 p-4">
      <output className="block text-sm text-muted">
        This window is under 900 px wide, so the canvas is read-only. Editing needs a wider window.
      </output>
      {nodes.length === 0 ? (
        <p className="text-sm">No components yet.</p>
      ) : (
        <ul className="flex flex-col gap-2">
          {nodes.map(([name, n]) => (
            <li key={name} className="rounded-lg border border-border px-3 py-2 text-sm">
              <span className="font-medium">{name}</span>
              <span className="text-muted">
                {" · "}
                {NODE_KINDS.find((k) => k.kind === n.type)?.label ?? String(n.type)}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
