// The storage node's own settings (a storage node refuses `proc`): its flavor (a label), the time to
// the first byte and the throughput that turns a body's size into time. Every edit is a patch.

import { useEffect, useId, useState } from "react";
import { loadStoragePrices } from "../../data/api/prices";
import type { NodeDoc } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { EDGE_LIMITS } from "./edgeFields";
import { NumberField } from "./Field";
import {
  carriesEstimates,
  firstByteEditable,
  firstByteMean,
  firstByteMs,
  firstByteSpread,
  FLAVORS,
  LIMITS,
  setDuration,
  setFirstByteMean,
  setFlavor,
  setSpread,
  setThroughput,
  throughputKb,
} from "./fields";
import { storageProvider } from "../cost/cost";
import {
  classOf,
  locationLabel,
  locationOf,
  retrievedOf,
  setClass,
  setLocation,
  setRetrieved,
  setStored,
  setWriteShare,
  STORAGE_CLASSES,
  storedOf,
  writeShareOf,
} from "./storageCost";
import { BYTE_SIZE_UNITS, RATE_UNITS, TIME_UNITS } from "./units";

const REMOVE =
  "mb-2 min-h-6 rounded-md border border-control px-2 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent";

export function StorageFields({ name, config }: { name: string; config: NodeDoc }) {
  const patch = useDraft((s) => s.patch);
  const flavorId = useId();
  const locationId = useId();
  const classId = useId();
  const provider = storageProvider(config);
  const [locations, setLocations] = useState<string[]>([]);
  useEffect(() => {
    let gone = false;
    loadStoragePrices().then(
      (p) =>
        !gone &&
        setLocations(
          Object.keys(p)
            .filter((k) => k.startsWith(`${provider}/`))
            .map((k) => k.slice(provider.length + 1))
            .filter((r) => r !== "*"),
        ),
      () => undefined,
    );
    return () => {
      gone = true;
    };
  }, [provider]);
  const location = locationOf(config);
  const flavor = typeof config.flavor === "string" ? config.flavor : "";
  const throughput = throughputKb(config);
  const spread = firstByteSpread(config);
  const mean = firstByteMean(config);
  return (
    <>
      <div className="mb-2">
        <label htmlFor={flavorId} className="block text-sm font-medium">
          Flavor
        </label>
        <select
          id={flavorId}
          value={flavor}
          aria-describedby={`${flavorId}-hint`}
          onChange={(e) => void patch([setFlavor(name, config, e.currentTarget.value)])}
          className="h-7 rounded border border-control bg-sheet text-text px-1"
        >
          {flavor === "" || !FLAVORS.some((f) => f === flavor) ? (
            <option value={flavor}>{flavor || "Not set"}</option>
          ) : null}
          {FLAVORS.map((f) => (
            <option key={f} value={f}>
              {f}
            </option>
          ))}
        </select>
        <p id={`${flavorId}-hint`} className="text-xs text-muted">
          A label. It sets no numbers.
        </p>
      </div>
      {firstByteEditable(config) ? (
        <NumberField
          label="First byte"
          help="first-byte"
          quietHint
          units={TIME_UNITS}
          hint="How long a call waits before the first byte arrives, whatever the size."
          value={firstByteMs(config)}
          min={LIMITS.serviceMs.min}
          max={LIMITS.serviceMs.max}
          step="any"
          onCommit={(n) => patch([setDuration(name, config, "first_byte", n)])}
        />
      ) : spread ? (
        (["p50", "p99"] as const).map((end) => (
          <NumberField
            key={end}
            label={`First byte ${end}`}
            help="first-byte"
            units={TIME_UNITS}
            hint={
              end === "p50"
                ? "Half of the calls wait this long or less before the first byte."
                : "Nearly all calls (99 in 100) wait this long or less. Not below the p50."
            }
            value={spread[end]}
            min={LIMITS.serviceMs.min}
            max={LIMITS.serviceMs.max}
            step="any"
            onCommit={(n) => patch(setSpread(name, config, end, n))}
          />
        ))
      ) : mean !== null ? (
        <NumberField
          label="First byte mean"
          help="first-byte"
          units={TIME_UNITS}
          hint="The average wait before the first byte; most calls are quicker, a few are much slower (an exponential spread)."
          value={mean}
          min={LIMITS.serviceMs.min}
          max={LIMITS.serviceMs.max}
          step="any"
          onCommit={(n) => patch(setFirstByteMean(name, n))}
        />
      ) : (
        <p className="mb-2 text-xs text-muted">
          First byte is written in a form the boxes cannot show. Edit it in kaanal.yaml.
        </p>
      )}
      <NumberField
        label="Throughput"
        help="throughput"
        quietHint
        units={RATE_UNITS}
        hint="How fast the body moves once it starts: request and reply bytes over this rate, added to the first byte. Empty means no size cost."
        value={throughput}
        placeholder="No limit"
        min={EDGE_LIMITS.bandwidthKb.min}
        max={EDGE_LIMITS.bandwidthKb.max}
        step="any"
        onCommit={(n) => patch(setThroughput(name, config, n))}
      />
      {throughput !== null ? (
        <button
          type="button"
          onClick={() => void patch(setThroughput(name, config, null))}
          className={REMOVE}
        >
          Remove the throughput
        </button>
      ) : null}
      <div className="mb-2">
        <label htmlFor={locationId} className="block text-sm font-medium">
          Location (for the cost)
        </label>
        <select
          id={locationId}
          value={location}
          aria-describedby={`${locationId}-hint`}
          onChange={(e) => void patch(setLocation(name, config, e.currentTarget.value))}
          className="h-7 max-w-full rounded border border-control bg-sheet px-1 text-text"
        >
          <option value="">The region picked at the globe</option>
          {location !== "" && !locations.includes(location) ? (
            <option value={location}>{location}</option>
          ) : null}
          {locations.map((l) => (
            <option key={l} value={l}>
              {locationLabel(l)}
            </option>
          ))}
        </select>
        <p id={`${locationId}-hint`} className="text-xs text-muted">
          Where this store is priced, instead of the globe's region. Google also offers dual-regions
          and multi-regions here. It changes no timing.
        </p>
      </div>
      <div className="mb-2">
        <label htmlFor={classId} className="block text-sm font-medium">
          Storage class (for the cost)
        </label>
        <select
          id={classId}
          value={classOf(config)}
          aria-describedby={`${classId}-hint`}
          onChange={(e) => void patch(setClass(name, config, e.currentTarget.value))}
          className="h-7 max-w-full rounded border border-control bg-sheet px-1 text-text"
        >
          {STORAGE_CLASSES.map((c) => (
            <option key={c.id} value={c.id}>
              {c.label}
            </option>
          ))}
        </select>
        <p id={`${classId}-hint`} className="text-xs text-muted">
          A colder class keeps data for less and charges more to use it: dearer requests and a fee
          for each GB read back. It changes no timing; write a slower first byte yourself.
        </p>
      </div>
      <NumberField
        label="Data kept (for the cost)"
        units={BYTE_SIZE_UNITS}
        hint="How much the store holds, priced per GB-month in the region you pick at the globe (S3 Standard, Azure Blob Hot, Google Cloud Storage as a placeholder). It changes no timing. Empty: only the requests are priced."
        value={storedOf(config)}
        placeholder="Not set"
        min={0}
        step="any"
        onCommit={(n) => patch(setStored(name, config, n))}
      />
      {classOf(config) !== "standard" ? (
        <NumberField
          label="Data read back a month (for the cost)"
          units={BYTE_SIZE_UNITS}
          hint="How much a colder class hands back each month: each GB is charged its retrieval fee. Empty: none."
          value={retrievedOf(config)}
          placeholder="Not set"
          min={0}
          step="any"
          onCommit={(n) => patch(setRetrieved(name, config, n))}
        />
      ) : null}
      <NumberField
        label="Share of calls that are writes"
        hint="0 to 1. A write (PUT, COPY, POST, LIST) costs more than a read (GET). Requests are priced from the visits of the last run. Empty: every call is priced as a read."
        value={writeShareOf(config)}
        placeholder="0 (all reads)"
        min={0}
        max={1}
        step="any"
        onCommit={(n) => patch(setWriteShare(name, config, n))}
      />
      {carriesEstimates(config) ? (
        <p className="mb-2 text-xs text-muted">
          These numbers are placeholders, not published figures. Providers publish request limits
          but not latencies.
        </p>
      ) : null}
    </>
  );
}
