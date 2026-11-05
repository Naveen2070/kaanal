// The Controls tab for one picked wire: what the link costs and carries. Latency is paid on every
// call over it; calls per request and the share of requests say how often it is used; bandwidth
// turns a payload into a transfer time on a link whose rate other transfers share. Every edit is a
// patch of the draft; the auto re-run follows.

import { ArrowRight } from "lucide-react";
import { useId } from "react";
import { useFeed } from "../../canvas/feed";
import { whyWireRed } from "../../canvas/insights";
import { removeWires } from "../../canvas/wires";
import { useDraft } from "../../data/store/draft";
import type { Doc, EdgeDoc, NodeDoc } from "../../data/api/types";
import { BalanceField, NumberField, Section } from "./Field";
import { balanceOf, setBalance } from "./fields";
import {
  bandwidthKbOf,
  BODY_MAX_BYTES,
  bodyBytesOf,
  callsOf,
  clearBandwidth,
  clearBody,
  clearCompress,
  clearKeepalive,
  COMPRESS_LIMITS,
  compressPctOf,
  EDGE_LIMITS,
  KEEPALIVE,
  LOSS_LIMITS,
  lossPctOf,
  keepaliveOf,
  netMsOf,
  setBandwidth,
  setBody,
  setCalls,
  setCompress,
  setKeepalive,
  setLoss,
  setNet,
  setShare,
  setStream,
  setTls,
  shareApplies,
  shareOf,
  STREAM_CHOICES,
  streamOf,
  type StreamChoice,
  TLS_CHOICES,
  tlsOf,
  type TlsChoice,
} from "./edgeFields";
import { BYTE_SIZE_UNITS, RATE_UNITS, rateText, TIME_UNITS } from "./units";

const callee = (doc: Doc | null | undefined, e: EdgeDoc): NodeDoc | undefined =>
  doc?.nodes?.[String(e.to)];

export function EdgeControls({ index }: { index: number }) {
  const doc = useDraft((s) => s.view?.doc);
  const patch = useDraft((s) => s.patch);
  const tlsId = useId();
  const streamId = useId();
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

      <Section title="What it carries">
        {(["sends", "returns"] as const).map((field) => (
          <div key={field}>
            <NumberField
              label={field === "sends" ? "Request size" : "Response size"}
              hint={
                field === "sends"
                  ? "The body each call over this wire carries. Empty means the size of the request that reached the caller."
                  : "The body each call over this wire brings back. Empty means the size of the response the caller itself gives."
              }
              value={bodyBytesOf(e, field)}
              units={BYTE_SIZE_UNITS}
              placeholder="Passed on"
              max={BODY_MAX_BYTES}
              step="any"
              onCommit={(n) => patch([setBody(index, field, e, n)])}
            />
            {field in e ? (
              <button
                type="button"
                onClick={() => void patch(clearBody(index, field, e))}
                className="mb-2 min-h-6 rounded-md border border-control px-2 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
              >
                {field === "sends" ? "Remove the request size" : "Remove the response size"}
              </button>
            ) : null}
          </div>
        ))}
        <NumberField
          label="Compress the response (%)"
          hint="The share of its size a response keeps on the wire when it is compressed (30 sends 30% of the bytes). Empty means not compressed. The component still handles the whole body."
          value={compressPctOf(e)}
          placeholder="Not compressed"
          min={COMPRESS_LIMITS.min}
          max={COMPRESS_LIMITS.max}
          step="any"
          onCommit={(n) => patch([setCompress(index, e, n)])}
        />
        {compressPctOf(e) !== null ? (
          <button
            type="button"
            onClick={() => void patch(clearCompress(index, e))}
            className="mb-2 min-h-6 rounded-md border border-control px-2 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
          >
            Stop compressing
          </button>
        ) : null}
        <div className="mb-2">
          <label htmlFor={streamId} className="block text-sm font-medium">
            Stream the response
          </label>
          <select
            id={streamId}
            value={streamOf(e)}
            aria-describedby={`${streamId}-hint`}
            onChange={(ev) =>
              void patch(setStream(index, e, ev.currentTarget.value as StreamChoice))
            }
            className="h-7 rounded border border-control bg-sheet px-1 text-text"
          >
            {STREAM_CHOICES.map((c) => (
              <option key={c.value} value={c.value}>
                {c.label}
              </option>
            ))}
          </select>
          <p id={`${streamId}-hint`} className="text-xs text-muted">
            A streamed body crosses the wire while the other end is still working on it (a download
            as the callee produces it, an upload as it arrives), so the call takes the first byte
            plus the size over the slowest stage, not the stages added. It only matters on a wire or
            component with a bandwidth.
          </p>
        </div>
      </Section>

      <Section title="Connection">
        <div className="mb-2">
          <label htmlFor={tlsId} className="block text-sm font-medium">
            Secure connection
          </label>
          <select
            id={tlsId}
            value={tlsOf(e)}
            aria-describedby={`${tlsId}-hint`}
            onChange={(ev) => void patch(setTls(index, e, ev.currentTarget.value as TlsChoice))}
            className="h-7 rounded border border-control bg-sheet px-1 text-text"
          >
            {TLS_CHOICES.map((c) => (
              <option key={c.value} value={c.value}>
                {c.label}
              </option>
            ))}
          </select>
          <p id={`${tlsId}-hint`} className="text-xs text-muted">
            A new connection pays one round trip for TCP, one more for TLS 1.3 or two for TLS 1.2. A
            round trip is this wire's latency, so a wire with no latency adds nothing.
          </p>
        </div>
        <NumberField
          label="Packet loss (%)"
          hint="The share of packets this wire drops. A lost packet costs a retransmit; a long transfer runs slower. Empty or 0 means no loss. It adds nothing on a wire with no latency."
          value={lossPctOf(e)}
          placeholder="No loss"
          min={LOSS_LIMITS.min}
          max={LOSS_LIMITS.max}
          step="any"
          onCommit={(n) => patch(setLoss(index, e, n))}
        />
        <NumberField
          label="Calls per connection"
          hint="How many calls one connection serves before it closes. One call in this many opens a new connection (handshake and slow start); 1 opens one every call."
          value={keepaliveOf(e)}
          placeholder={`${KEEPALIVE.default} (default)`}
          min={KEEPALIVE.min}
          max={KEEPALIVE.max}
          integer
          onCommit={(n) => patch([setKeepalive(index, e, n)])}
        />
        {"keepalive" in e ? (
          <button
            type="button"
            onClick={() => void patch(clearKeepalive(index, e))}
            className="mb-2 min-h-6 rounded-md border border-control px-2 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
          >
            Back to the default
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
