// The right-hand panel: the selected node's tabs. Controls is built; Log and Results arrive with
// Part E and say so rather than sit empty.

import { useDraft } from "../store/draft";
import { runNow } from "../store/loopRuntime";
import { TABS, useSelected, useTab, type Tab } from "../store/view";
import { Controls } from "./Controls";
import { LogTab } from "./LogTab";
import { Results } from "./Results";

const LABEL: Record<Tab, string> = { controls: "Controls", log: "Log", results: "Results" };

export function Panel() {
  const view = useDraft((s) => s.view);
  const [selected, setSelected] = useSelected();
  const [tab, setTab] = useTab();
  const config = selected !== null ? view?.doc?.nodes?.[selected] : undefined;

  return (
    <aside
      data-tour="panel"
      aria-label="Details"
      className="flex w-80 flex-col overflow-y-auto border-l"
    >
      <div role="tablist" className="flex border-b">
        {TABS.map((t) => (
          <button
            key={t}
            role="tab"
            type="button"
            aria-selected={tab === t}
            onClick={() => void setTab(t)}
            className={`flex-1 px-2 py-1 text-sm ${tab === t ? "border-b-2 border-blue-500 font-medium" : ""}`}
          >
            {LABEL[t]}
          </button>
        ))}
      </div>
      {tab === "log" ? (
        <LogTab />
      ) : tab === "results" ? (
        <Results />
      ) : selected !== null && config ? (
        <Controls
          name={selected}
          config={config}
          edges={view?.doc?.edges ?? []}
          onDeleted={() => void setSelected(null)}
          onRenamed={(to) => void setSelected(to)}
          onChaos={runNow}
        />
      ) : (
        <p className="p-3 text-sm text-neutral-500">Select a component to see and change it.</p>
      )}
    </aside>
  );
}
