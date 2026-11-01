// Under 768 px the canvas is read-only: the nodes as a list, each with its state in words from the
// newest run, and the reason.

import { useFeed } from "../canvas/feed";
import { nodeFigures } from "../canvas/metrics";
import { NODE_KINDS } from "../canvas/nodeKinds";
import { useDraft } from "../store/draft";

export function ReadonlyNodes() {
  const nodes = Object.entries(useDraft((s) => s.view?.doc?.nodes) ?? {});
  const feed = useFeed();
  return (
    <section aria-label="Components" className="flex flex-col gap-3 p-4">
      <output className="block text-sm text-muted">
        Open this on a wider screen (768 px or more) to edit. Here is how each part is doing.
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
              <span className="block text-xs text-muted">
                {nodeFigures(feed.node(name), n).sentence}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
