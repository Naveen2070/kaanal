// What a component's live queue and drops look like (step 8L): requests that cannot start yet
// stack up as a pile of dots at the foot of the card, and a drop flashes a cross. Both say the same
// in words for assistive technology ("3 waiting", "2 turned away"); colour is never the only carrier.

import { useDraft } from "../../data/store/draft";
import { useLive } from "../../data/store/live";
import { usePrefersReducedMotion } from "../../overlays/first-run/motion";
import { pileSize } from "./dots";

export function LivePile({ name }: { name: string }) {
  const still = usePrefersReducedMotion();
  // the tick lists components in file order: the stream says which, the draft JSON is alphabetical
  const order = useLive((s) => s.hello?.nodes);
  const docNames = useDraft((s) => Object.keys(s.view?.doc?.nodes ?? {}).join("\u0000"));
  const i = (order ?? docNames.split("\u0000")).indexOf(name);
  const tick = useLive((s) => s.ticks.at(-1));
  const n = tick?.nodes[i];
  if (!n) return null;
  const waiting = n.queue;
  const d = n.dropped;
  const dropped = d.overflow + d.rate_limit + d.deadline + d.breaker + d.chaos;
  if (waiting === 0 && dropped === 0) return null;
  return (
    <div className="flex h-4 items-center gap-1 text-[11px] text-muted" data-part="live">
      {waiting > 0 ? (
        <>
          <span aria-hidden className="flex gap-0.5" data-pile>
            {Array.from({ length: pileSize(waiting) }, (_, k) => (
              <span key={k} className="size-1.5 rounded-full bg-muted" />
            ))}
          </span>
          <span>{waiting} waiting</span>
        </>
      ) : null}
      {dropped > 0 ? (
        <span
          // the tick is the key, so a new drop restarts the flash
          key={tick?.t_ms}
          data-flash
          className={`font-semibold text-negative ${still ? "" : "kaanal-flash"}`}
        >
          <span aria-hidden>×</span> {dropped} turned away
        </span>
      ) : null}
    </div>
  );
}
