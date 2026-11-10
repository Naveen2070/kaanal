// The instance picker of a component's CPUs group (steps 8P, RULES 113): a cloud type from the table
// of AWS, Azure and GCP (a provider, then a search over its types), or one of the file's own
// instance types for a machine the table does not have (a self-hosted server, another provider).
// Choosing one writes `instance:` on the node; a custom one is an entry of the top-level
// `instances:` map, edited here.

import { useEffect, useId, useState } from "react";
import {
  PROVIDERS,
  findInstance,
  instanceLine,
  loadInstances,
  matching,
  type InstanceRow,
} from "../../data/api/instances";
import type { NodeDoc } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { FieldLabel, NumberField } from "./Field";
import {
  CUSTOM_NAME,
  addCustomInstance,
  customInstancesOf,
  instanceOf,
  removeCustomInstance,
  setCustomInstance,
  setInstance,
  usersOfInstance,
} from "./fields";
import { BYTE_SIZE_UNITS, RATE_UNITS } from "./units";

/** `""` none, `custom` your own, or a provider id. */
type Mode = string;

const SELECT = "h-7 max-w-full rounded border border-control bg-sheet px-1 text-text";
const INPUT = "h-7 rounded border border-control bg-sheet px-2 text-text";
const BTN =
  "h-7 rounded border border-control bg-sheet px-2 text-sm hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-50";

export function InstancePicker({ name, config }: { name: string; config: NodeDoc }) {
  const patch = useDraft((s) => s.patch);
  const doc = useDraft((s) => s.view?.doc ?? null);
  const id = useId();
  const now = instanceOf(config);
  const mine = customInstancesOf(doc);
  const [rows, setRows] = useState<InstanceRow[] | null>(null);
  const [failed, setFailed] = useState(false);
  const [query, setQuery] = useState("");
  const [newName, setNewName] = useState("");
  const [newCores, setNewCores] = useState("");
  const [problem, setProblem] = useState<string | null>(null);

  const current: Mode = now === null ? "" : now in mine ? "custom" : (now.split("/")[0] ?? "");
  const [chosen, setChosen] = useState<Mode | null>(null);
  const mode: Mode = chosen ?? current;
  // the table is fetched when a cloud provider is in play, not for every panel that opens
  const needTable = PROVIDERS.some((p) => p.id === mode);

  useEffect(() => {
    if (!needTable) return;
    let gone = false;
    loadInstances().then(
      (r) => !gone && setRows(r),
      () => !gone && setFailed(true),
    );
    return () => {
      gone = true;
    };
  }, [needTable]);
  const row = rows && now !== null ? findInstance(rows, now) : undefined;
  const custom = now !== null && now in mine ? mine[now] : undefined;

  const onMode = (m: Mode) => {
    setChosen(m);
    setQuery("");
    setProblem(null);
    if (m === "") void patch(setInstance(name, config, null));
  };

  const pick = (text: string) => {
    setQuery(text);
    const hit = rows
      ? findInstance(rows, text.includes("/") ? text : `${mode}/${text}`)
      : undefined;
    if (hit) void patch(setInstance(name, config, hit.name));
  };

  const create = async () => {
    const n = newName.trim();
    const cores = Number(newCores);
    if (!CUSTOM_NAME.test(n)) {
      return setProblem("A name has letters, digits, . _ or - (no /), up to 64 characters.");
    }
    if (n in mine) return setProblem(`'${n}' is already one of your instances.`);
    if (!(cores >= 0.001 && cores <= 4096)) {
      return setProblem("Cores: a number from 0.001 to 4096.");
    }
    if (!(await patch(addCustomInstance(doc, n, cores)))) {
      return setProblem("The draft refused the change.");
    }
    setProblem(null);
    setNewName("");
    setNewCores("");
    await patch(setInstance(name, config, n));
  };

  const options = rows && PROVIDERS.some((p) => p.id === mode) ? matching(rows, mode, query) : [];

  return (
    <div className="mb-2">
      <FieldLabel htmlFor={id} label="Instance type" />
      <select
        id={id}
        value={mode}
        aria-describedby={`${id}-hint`}
        onChange={(e) => onMode(e.currentTarget.value)}
        className={SELECT}
      >
        <option value="">None</option>
        {PROVIDERS.map((p) => (
          <option key={p.id} value={p.id}>
            {p.label}
          </option>
        ))}
        <option value="custom">Your own</option>
      </select>
      {PROVIDERS.some((p) => p.id === mode) ? (
        <div className="mt-1">
          <input
            aria-label="Search instance types"
            list={`${id}-types`}
            value={query || (now !== null && now.startsWith(`${mode}/`) ? now : "")}
            placeholder={failed ? "The table did not load" : "m5.large, n2-standard-4, d4s_v5 ..."}
            disabled={rows === null}
            onChange={(e) => pick(e.currentTarget.value)}
            className={`${INPUT} w-64 max-w-full font-mono`}
          />
          <datalist id={`${id}-types`}>
            {options.map((o) => (
              <option key={o.name} value={o.name}>
                {instanceLine(o)}
              </option>
            ))}
          </datalist>
        </div>
      ) : null}
      {mode === "custom" ? (
        <div className="mt-1 flex flex-col gap-1">
          <select
            aria-label="Your instance types"
            value={custom ? (now ?? "") : ""}
            onChange={(e) => void patch(setInstance(name, config, e.currentTarget.value || null))}
            className={SELECT}
          >
            <option value="">Choose one of yours ({Object.keys(mine).length})</option>
            {Object.keys(mine).map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
          <div className="flex flex-wrap items-end gap-1">
            <input
              aria-label="New instance name"
              placeholder="name, e.g. build-box"
              value={newName}
              onChange={(e) => setNewName(e.currentTarget.value)}
              className={`${INPUT} w-40`}
            />
            <input
              aria-label="New instance cores"
              inputMode="decimal"
              placeholder="cores"
              value={newCores}
              onChange={(e) => setNewCores(e.currentTarget.value)}
              className={`${INPUT} w-20`}
            />
            <button type="button" onClick={() => void create()} className={BTN}>
              Add instance
            </button>
          </div>
          {problem ? (
            <p role="alert" className="m-0 text-xs text-negative">
              {problem}
            </p>
          ) : null}
        </div>
      ) : null}
      {custom && now !== null ? (
        <div className="mt-1">
          <NumberField
            label={`${now}: cores`}
            hint="CPUs of this instance. Every node that names it, and does not set its own cores, gets these."
            value={custom.cores}
            min={0.001}
            max={4096}
            step="any"
            onCommit={(n) => patch(setCustomInstance(doc, now, "cores", n))}
          />
          <NumberField
            label={`${now}: memory`}
            hint="The memory limit of a node that sets a memory and leaves its limit out. 0 = none."
            value={custom.memory}
            units={BYTE_SIZE_UNITS}
            placeholder="None"
            step="any"
            onCommit={(n) => patch(setCustomInstance(doc, now, "memory", n))}
          />
          <NumberField
            label={`${now}: network`}
            hint="The network card, both ways, of a node that sets none. 0 = not limited."
            value={custom.bandwidth === null ? null : custom.bandwidth / 1024}
            units={RATE_UNITS}
            placeholder="Not limited"
            step="any"
            onCommit={(n) => patch(setCustomInstance(doc, now, "bandwidth", n * 1024))}
          />
          <button
            type="button"
            disabled={usersOfInstance(doc, now).length > 0}
            onClick={() => void patch(removeCustomInstance(doc, now))}
            className={BTN}
            title={
              usersOfInstance(doc, now).length > 0
                ? `Used by ${usersOfInstance(doc, now).join(", ")}`
                : "Delete this instance"
            }
          >
            Delete {now}
          </button>
        </div>
      ) : null}
      <p id={`${id}-hint`} className="mt-1 text-xs text-muted">
        {row
          ? `${instanceLine(row)}. Fills what you leave empty; what you set wins. Estimates: a vCPU counts as a full core, so this is an upper bound on capacity, not on speed.`
          : custom
            ? "Your own instance: fills the cores, network card and memory limit you leave empty."
            : "Fills the cores, the network card and the memory limit you leave empty: a cloud instance type (AWS, Azure, Google Cloud; estimates) or one you define yourself."}
      </p>
    </div>
  );
}
