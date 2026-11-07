// The headers every call carries on top of its body (`header_bytes:` at the top of the file). The
// numbers are labelled estimates of a browser's and a server's headers; 0 on both turns them off.

import { useDraft } from "../../data/store/draft";
import { NumberField } from "./Field";
import { Group } from "./groups/Group";
import { DOC_GROUP } from "./groups/ids";
import { headersSummary } from "./groups/summaries";
import { headerBytesOf, setHeaderBytes } from "./edgeFields";
import { BYTE_SIZE_UNITS } from "./units";

export function HeaderBytes() {
  const doc = useDraft((s) => s.view?.doc);
  const patch = useDraft((s) => s.patch);
  if (!doc) return null;
  const h = headerBytesOf(doc);
  return (
    <Group
      id={DOC_GROUP.headers}
      title="Headers on every call"
      summary={headersSummary(doc)}
      changed={"header_bytes" in doc}
      onReset={() => void patch([{ op: "remove", path: ["header_bytes"] }])}
    >
      <NumberField
        label="Request headers"
        help="header-bytes"
        quietHint
        hint="An estimate of what a client sends besides the body (cookies and a token included). 0 on both turns headers off."
        value={h.request}
        units={BYTE_SIZE_UNITS}
        max={1024 ** 2}
        step="any"
        onCommit={(n) => patch([setHeaderBytes(doc, "request", n)])}
      />
      <NumberField
        label="Response headers"
        help="header-bytes"
        quietHint
        hint="An estimate of what a server answers besides the body."
        value={h.response}
        units={BYTE_SIZE_UNITS}
        max={1024 ** 2}
        step="any"
        onCommit={(n) => patch([setHeaderBytes(doc, "response", n)])}
      />
    </Group>
  );
}
