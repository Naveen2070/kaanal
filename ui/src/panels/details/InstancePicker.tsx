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
import { regionOf, useRegion, useRegionTable } from "../../data/store/region";
import {
  hasManagedClasses,
  isManagedClass,
  isManagedService,
  isOwnClass,
  priceListKey,
} from "./hosting";
import { FieldLabel, NumberField } from "./Field";
import {
  CUSTOM_NAME,
  addCustomInstance,
  customInstancesOf,
  instanceOf,
  removeCustomInstance,
  setCores,
  setCustomInstance,
  setInstance,
  estimatePerRequest,
  memoryOf,
  setMemoryFields,
  slotsOf,
  usersOfInstance,
} from "./fields";
import { notify } from "../../shell/toast/toasts";
import { BYTE_SIZE_UNITS, RATE_UNITS } from "./units";

/** "128 MiB", "1 GiB": a size in bytes for a sentence. */
function bytesText(b: number): string {
  const gib = b / 1024 ** 3;
  if (gib >= 1) return `${Number(gib.toFixed(2))} GiB`;
  return `${Number((b / 1024 ** 2).toFixed(1))} MiB`;
}

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

  // choosing a type writes it, fills Cores with its vCPUs and the memory limit with its memory.
  // A limit alone is refused ("memory: per_request is required"), so when the node has no
  // per-request size one is estimated: (memory - base) / slots, the size at which memory and
  // slots run out together. It is said to be an estimate, here and in a notice; a size you set
  // yourself is never replaced.
  // the warning is read from the draft, not remembered: it shows for as long as the size per
  // request is the one this rule gives, so a reload or another panel does not lose it
  const mem0 = memoryOf(config);
  const slots0 = slotsOf(config).value;
  const estimated =
    now !== null &&
    mem0.limit !== null &&
    mem0.per_request !== null &&
    mem0.per_request === estimatePerRequest(mem0.limit, mem0.base ?? 0, slots0);
  const choose = async (inst: string, cores: number, memBytes: number | null) => {
    // a cache's hits are fixed times: its instance is a price and a size note, so no cores or memory limit
    if (config.type === "cache") {
      await patch(setInstance(name, config, inst));
      return;
    }
    const mem = memoryOf(config);
    const slots = slotsOf(config).value;
    const guess =
      memBytes && mem.per_request === null
        ? estimatePerRequest(memBytes, mem.base ?? 0, slots)
        : null;
    const fields =
      memBytes && (mem.per_request !== null || guess !== null)
        ? setMemoryFields(name, config, {
            limit: memBytes,
            ...(guess !== null && { per_request: guess }),
          })
        : [];
    const ok = await patch([
      ...setInstance(name, config, inst),
      ...setCores(name, config, cores),
      ...fields,
    ]);
    if (ok && guess !== null && memBytes) {
      notify.warn(
        `Memory per request was set to ${bytesText(guess)}, an estimate and not a measurement: (${bytesText(memBytes)} - base) / ${slots} slots. Change it to what a request really holds.`,
        "memory-estimate",
      );
    }
  };

  const pick = (text: string) => {
    setQuery(text);
    const hit = rows
      ? findInstance(rows, text.includes("/") ? text : `${mode}/${text}`)
      : undefined;
    if (hit) void choose(hit.name, hit.vcpu, hit.memMib * 1024 * 1024);
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
    await patch(setCores(name, config, cores));
  };

  // only the types the picked region has (a type with a price there is offered there); every type
  // when the region's list could not be loaded
  const provider = PROVIDERS.some((p) => p.id === mode) ? mode : null;
  // a managed database lists the provider's database classes (priced per engine), every other
  // node its VM types
  const classes = isManagedService(config);
  const offered = useRegionTable(
    provider,
    provider === null ? null : priceListKey(config, provider),
  );
  const region = useRegion((s) => (provider === null ? undefined : regionOf(s, provider)));
  const options = (rows && provider !== null ? matching(rows, provider, query, 2000) : [])
    .filter((o) =>
      classes
        ? hasManagedClasses(config) && isOwnClass(config.type, o.name)
        : !isManagedClass(o.name),
    )
    .filter((o) => !offered || offered.has(o.name))
    .slice(0, 200);
  const missingHere =
    offered && now !== null && now.startsWith(`${mode}/`) && !offered.has(now.toLowerCase());

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
        {PROVIDERS.filter((p) => !classes || p.id === "aws").map((p) => (
          <option key={p.id} value={p.id}>
            {classes
              ? `${p.label} (managed ${config.type === "cache" ? "cache" : "database"} classes)`
              : p.label}
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
            placeholder={
              failed
                ? "The table did not load"
                : classes
                  ? config.type === "cache"
                    ? "cache.m5.large, cache.r6g.large ..."
                    : "db.m5.large, db.r6g.xlarge ..."
                  : "m5.large, n2-standard-4, d4s_v5 ..."
            }
            disabled={rows === null}
            onChange={(e) => pick(e.currentTarget.value)}
            className={`${INPUT} w-64 max-w-full font-mono`}
          />
          <p className="m-0 mt-1 text-xs text-muted">
            Region {region ?? "…"} (the globe at the top right changes it):{" "}
            {offered
              ? `${offered.size.toLocaleString("en-US")} types offered here`
              : "loading the types offered here"}
            .
          </p>
          {missingHere ? (
            <p className="m-0 mt-1 text-xs text-warn">
              {now} has no price listed in {region}: it may not be offered there. Pick another type
              or region.
            </p>
          ) : null}
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
            onChange={(e) => {
              const n = e.currentTarget.value;
              const c = mine[n];
              if (!n || !c) return void patch(setInstance(name, config, null));
              void choose(n, c.cores ?? 1, c.memory);
            }}
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
          <NumberField
            label={`${now}: cost per hour (USD)`}
            hint="What one hour of this machine costs, for the cost estimate under the node list. Empty = no price: the estimate leaves it out and says so."
            value={custom.cost}
            placeholder="No price"
            min={0}
            step="any"
            onCommit={(n) => patch(setCustomInstance(doc, now, "cost_per_hour", n))}
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
      {estimated && mem0.limit !== null && mem0.per_request !== null ? (
        <output className="mt-1 block rounded border border-warn px-2 py-1.5 text-xs text-warn">
          <strong>Memory per request is an estimate, not a measured value.</strong> Studio set it to{" "}
          {bytesText(mem0.per_request)} = (memory {bytesText(mem0.limit)} - base) / {slots0} slots:
          the size at which memory and slots run out together, so memory never limits you before the
          slots do. A limit needs a size per request, so Studio fills one to keep the draft valid.
          Real services hold more or less: set Memory per request to what yours holds.
        </output>
      ) : null}
      <p id={`${id}-hint`} className="mt-1 text-xs text-muted">
        {row
          ? `${instanceLine(row)}. Fills Cores and the memory limit (change them if you like); with no memory per request set, one is estimated from the memory and the slots. Estimates: a vCPU counts as a full core, so this is an upper bound on capacity, not on speed.`
          : custom
            ? "Your own instance: fills Cores and the memory limit (with an estimated size per request when none is set), and the network card when you leave it empty."
            : "Fills Cores and the memory limit, with an estimated size per request when none is set: a cloud instance type (AWS, Azure, Google Cloud; estimates) or one you define yourself."}
      </p>
    </div>
  );
}
