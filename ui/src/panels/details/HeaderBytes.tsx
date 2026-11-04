// The headers every call carries on top of its body (`header_bytes:` at the top of the file). The
// numbers are labelled estimates of a browser's and a server's headers; 0 on both turns them off.

import { useDraft } from "../../data/store/draft";
import { NumberField } from "./Field";
import { headerBytesOf, setHeaderBytes } from "./edgeFields";
import { BYTE_SIZE_UNITS } from "./units";

export function HeaderBytes() {
  const doc = useDraft((s) => s.view?.doc);
  const patch = useDraft((s) => s.patch);
  if (!doc) return null;
  const h = headerBytesOf(doc);
  return (
    <section className="mt-4 border-t border-rule pt-3" aria-label="Headers">
      <h3 className="m-0 mb-1 text-xs font-medium text-muted">Headers on every call</h3>
      <NumberField
        label="Request headers"
        hint="An estimate of what a client sends besides the body (cookies and a token included). 0 on both turns headers off."
        value={h.request}
        units={BYTE_SIZE_UNITS}
        max={1024 ** 2}
        step="any"
        onCommit={(n) => patch([setHeaderBytes(doc, "request", n)])}
      />
      <NumberField
        label="Response headers"
        hint="An estimate of what a server answers besides the body."
        value={h.response}
        units={BYTE_SIZE_UNITS}
        max={1024 ** 2}
        step="any"
        onCommit={(n) => patch([setHeaderBytes(doc, "response", n)])}
      />
    </section>
  );
}
