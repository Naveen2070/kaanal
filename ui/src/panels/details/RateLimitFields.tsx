// A component's request limit (`rate_limit`: `rps` and `burst`), for every kind: how many requests a
// second it accepts, with the rest turned away (a 503 at a storage node, a 429 anywhere else, as the
// engine does). Absent means no limit. Every edit is a patch.

import type { NodeDoc } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { NumberField } from "./Field";
import { addRateLimit, LIMITS, rateLimitOf, setRateLimit } from "./fields";

const BUTTON =
  "mb-2 min-h-6 rounded-md border border-control px-2 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent";

export function RateLimitFields({ name, config }: { name: string; config: NodeDoc }) {
  const patch = useDraft((s) => s.patch);
  const limit = rateLimitOf(config);
  const store = config.type === "storage";
  return limit ? (
    <>
      <NumberField
        label="Request limit"
        hint={
          store
            ? "Requests a second this store accepts; the rest are turned away with a 503."
            : "Requests a second this component accepts; the rest are turned away with a 429 (Too Many Requests)."
        }
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
        className={BUTTON}
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
        className={BUTTON}
      >
        Add a request limit
      </button>
      <p className="mb-2 text-xs text-muted">
        {store
          ? "Starts at 5,500 a second, the figure S3 publishes for reads on one prefix. Change it to your store's."
          : "Starts at 1,000 a second, a placeholder and not a published figure. Change it to yours."}
      </p>
    </>
  );
}
