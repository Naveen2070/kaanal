// What this tab is connected to: the project, the server, where the draft stands, and the control
// token (masked, with Show and Copy) for the kaanal command line. The token never leaves this tab:
// Copy puts it on the clipboard and nothing is sent anywhere. Shown inside the common menu.

import { Check, Copy, Eye, EyeOff } from "lucide-react";
import { useEffect, useState } from "react";
import { useDraft } from "../store/draft";
import { readToken } from "../token";

const MASK = "••••••••••••";

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-3 py-1.5">
      <dt className="shrink-0 text-xs text-muted">{label}</dt>
      <dd className="m-0 min-w-0 truncate text-right text-sm">{children}</dd>
    </div>
  );
}

export function ProfileView() {
  const view = useDraft((s) => s.view);
  const token = readToken();
  const [shown, setShown] = useState(false);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const t = setTimeout(() => setCopied(false), 2000);
    return () => clearTimeout(t);
  }, [copied]);

  const copy = () => {
    void navigator.clipboard?.writeText(token).then(() => setCopied(true));
  };
  const dirty = view?.dirty ?? false;
  const id = view?.project_id && view.project_id !== "none" ? view.project_id : null;
  const btn =
    "flex h-8 items-center gap-1.5 rounded-lg border border-control px-3 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent";

  return (
    <>
      <dl className="m-0 mt-2 divide-y divide-rule">
        <Row label="Project">{view?.project_name ?? "No project file yet"}</Row>
        <Row label="Project id">
          {id ? (
            <span className="font-mono text-xs">{id}</span>
          ) : (
            <span className="text-muted">–</span>
          )}
        </Row>
        <Row label="Server">
          <span className="font-mono text-xs">{location.origin}</span>
        </Row>
        <Row label="Draft">
          {`${dirty ? "not applied" : "applied"} · ${view?.saved === false ? "not saved" : "saved"}`}
        </Row>
        <Row label="Control token">
          <span className="font-mono text-xs">
            {token === "" ? "missing" : shown ? token : MASK}
          </span>
        </Row>
      </dl>
      {token !== "" ? (
        <div className="mt-2 flex gap-2">
          <button type="button" onClick={() => setShown((s) => !s)} className={btn}>
            {shown ? <EyeOff size={14} aria-hidden /> : <Eye size={14} aria-hidden />}
            {shown ? "Hide" : "Show"}
          </button>
          <button type="button" onClick={copy} className={btn}>
            {copied ? <Check size={14} aria-hidden /> : <Copy size={14} aria-hidden />}
            {copied ? "Copied" : "Copy"}
          </button>
        </div>
      ) : null}
      <p className="m-0 mt-3 text-xs text-muted">
        For the kaanal command line, set KAANAL_ENDPOINT to the server and KAANAL_TOKEN to the
        token.
      </p>
    </>
  );
}
