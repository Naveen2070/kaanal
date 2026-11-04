// The storage node's own settings (a storage node refuses `proc`): its flavor (a label), the time to
// the first byte and the throughput that turns a body's size into time. Every edit is a patch.

import { useId } from "react";
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
  addRateLimit,
  LIMITS,
  rateLimitOf,
  setDuration,
  setFirstByteMean,
  setFlavor,
  setRateLimit,
  setSpread,
  setThroughput,
  throughputKb,
} from "./fields";
import { RATE_UNITS, TIME_UNITS } from "./units";

const REMOVE =
  "mb-2 min-h-6 rounded-md border border-control px-2 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent";

export function StorageFields({ name, config }: { name: string; config: NodeDoc }) {
  const patch = useDraft((s) => s.patch);
  const flavorId = useId();
  const flavor = typeof config.flavor === "string" ? config.flavor : "";
  const throughput = throughputKb(config);
  const spread = firstByteSpread(config);
  const mean = firstByteMean(config);
  const limit = rateLimitOf(config);
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
      {limit ? (
        <>
          <NumberField
            label="Request limit"
            hint="Requests a second this store accepts; the rest are turned away with a 503."
            value={limit.rps}
            min={LIMITS.rate.min}
            max={LIMITS.rate.max}
            step="any"
            onCommit={(n) => patch(setRateLimit(name, config, "rps", n))}
          />
          <NumberField
            label="Burst"
            hint="The most requests let through at once after a quiet spell."
            value={limit.burst}
            min={1}
            max={LIMITS.rate.max}
            step={1}
            onCommit={(n) => patch(setRateLimit(name, config, "burst", n))}
          />
          <button
            type="button"
            onClick={() => void patch(setRateLimit(name, config, null))}
            className={REMOVE}
          >
            Remove the request limit
          </button>
        </>
      ) : "rate_limit" in config ? (
        <p className="mb-2 text-xs text-muted">
          A request limit is set in a form the boxes cannot show. Edit it in kaanal.yaml.
        </p>
      ) : (
        <>
          <button
            type="button"
            onClick={() => void patch(addRateLimit(name, config))}
            className={REMOVE}
          >
            Add a request limit
          </button>
          <p className="mb-2 text-xs text-muted">
            Starts at 5,500 a second, the figure S3 publishes for reads on one prefix. Change it to
            your store's.
          </p>
        </>
      )}
      {carriesEstimates(config) ? (
        <p className="mb-2 text-xs text-muted">
          These numbers are placeholders, not published figures. Providers publish request limits
          but not latencies.
        </p>
      ) : null}
    </>
  );
}
