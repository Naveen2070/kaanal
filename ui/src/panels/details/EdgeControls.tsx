// The Controls tab for one picked wire: what the link costs and carries. Latency is paid on every
// call over it; calls per request and the share of requests say how often it is used; bandwidth
// turns a payload into a transfer time on a link other transfers queue behind. Every edit is a
// patch of the draft; the auto re-run follows.

import { ArrowRight } from "lucide-react";
import { useFeed } from "../../canvas/feed";
import { whyWireRed } from "../../canvas/insights";
import { removeWires } from "../../canvas/wires";
import { useDraft } from "../../data/store/draft";
import type { Doc, EdgeDoc, NodeDoc } from "../../data/api/types";
import { BalanceField, NumberField, Section } from "./Field";
import { balanceOf, setBalance } from "./fields";
import {
  bandwidthKbOf,
  callsOf,
  clearBandwidth,
  EDGE_LIMITS,
  netMsOf,
  setBandwidth,
  setCalls,
  setNet,
  setShare,
  shareApplies,
  shareOf,
} from "./edgeFields";
import { RATE_UNITS, rateText, TIME_UNITS } from "./units";

const callee = (doc: Doc | null | undefined, e: EdgeDoc): NodeDoc | undefined =>
  doc?.nodes?.[String(e.to)];

export function EdgeControls({ index }: { index: number }) {
  const doc = useDraft((s) => s.view?.doc);
  const patch = useDraft((s) => s.patch);
  const feed = useFeed();
  const e = doc?.edges?.[index];
  if (!e) return null;
  const net = netMsOf(e);
  const bandwidth = bandwidthKbOf(e);
  // a full wire: the load asks for offered_load times what it carries
  const link = feed.link?.(String(e.from), String(e.to));
  const why = link ? whyWireRed(link, bandwidth) : null;
  const needKb =
    link?.saturated && bandwidth !== null && link.offered_load > 1
      ? bandwidth * link.offered_load
      : null;
  return (
    <div className="p-3 text-sm">
      <Section title="This wire">
        <p className="flex items-center gap-1.5 font-medium">
          {String(e.from)}
          <ArrowRight size={14} aria-hidden className="text-muted" />
          {String(e.to)}
        </p>
        <p className="mt-1 text-xs text-muted">
          {String(e.from)} calls {String(e.to)} over this wire.
        </p>
      </Section>

      <Section title="What it costs">
        {net.spread ? (
          <p className="mb-2 text-xs text-muted">
            Latency is a spread of values (for example p50 and p99). Edit it in kaanal.yaml for now.
          </p>
        ) : (
          <NumberField
            label="Network latency"
            help="time"
            hint="Time on the wire for each call, before the callee starts. Empty means none."
            value={net.ms}
            units={TIME_UNITS}
            placeholder="None"
            min={EDGE_LIMITS.netMs.min}
            max={EDGE_LIMITS.netMs.max}
            step="any"
            onCommit={(n) => patch([setNet(index, e, n)])}
          />
        )}
        <NumberField
          label="Bandwidth"
          hint="What the link carries each second; empty means no limit."
          value={bandwidth}
          units={RATE_UNITS}
          placeholder="No limit"
          min={EDGE_LIMITS.bandwidthKb.min}
          max={EDGE_LIMITS.bandwidthKb.max}
          step="any"
          onCommit={(n) => patch([setBandwidth(index, e, n)])}
        />
        {needKb !== null ? (
          <p className="mb-2 text-xs text-negative">
            Full at this load: it needs at least {rateText(needKb)}.
          </p>
        ) : null}
        {link && why ? <p className="mb-2 text-xs text-muted">{why}</p> : null}
        {bandwidth !== null ? (
          <button
            type="button"
            onClick={() => void patch(clearBandwidth(index, e))}
            className="mb-2 min-h-6 rounded-md border border-control px-2 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
          >
            Remove the bandwidth limit
          </button>
        ) : null}
      </Section>

      <Section title="How often it is used">
        {Number(callee(doc, e)?.replicas ?? 1) > 1 ? (
          <BalanceField
            hint="How calls over this wire pick a copy of the callee. It overrides the callee's own setting."
            value={balanceOf(e.balance)}
            defaultLabel="Same as the component (default)"
            overrides
            onChange={(c) => void patch(setBalance(["edges", index, "balance"], "balance" in e, c))}
          />
        ) : null}
        <NumberField
          label="Calls per request"
          hint="How many times one request calls over this wire (a chain's own repeat wins)."
          value={callsOf(e)}
          min={EDGE_LIMITS.calls.min}
          max={EDGE_LIMITS.calls.max}
          integer
          onCommit={(n) => patch([setCalls(index, e, n)])}
        />
        {shareApplies(doc, e) ? (
          <NumberField
            label="Share of requests (%)"
            hint="The part of requests that take this wire. A step in a chain can say its own."
            value={shareOf(e)}
            min={EDGE_LIMITS.sharePct.min}
            max={EDGE_LIMITS.sharePct.max}
            step="any"
            onCommit={(n) => patch([setShare(index, e, n)])}
          />
        ) : (
          <p className="mb-2 text-xs text-muted">
            A share of requests is not set on a wire out of a gateway (the routes decide) or into a
            cache (it keeps its own hit ratio).
          </p>
        )}
      </Section>

      <button
        type="button"
        className="rounded border border-negative px-2 py-1 text-negative"
        onClick={() => void removeWires([index])}
      >
        Delete wire
      </button>
    </div>
  );
}
