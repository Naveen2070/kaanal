// Which component owns each route of the draft (B8). Only the server knows: it builds the draft and
// the validate answer says it. Asked again, a moment after the text settles; a draft that does not
// build has no owners (the Steps group then shows nothing, and the problem is already on screen).

import { useEffect, useState } from "react";
import { api } from "../api/client";
import { useDraft } from "./draft";

const NONE: Readonly<Record<string, string>> = {};
const SETTLE_MS = 400;

/** Route key to owner node name, for the draft as it is now; nothing is asked while `enabled` is false. */
export function useRouteOwners(enabled = true): Readonly<Record<string, string>> {
  const text = useDraft((s) => s.view?.text);
  const [owners, setOwners] = useState(NONE);
  useEffect(() => {
    if (!enabled || text === undefined) return;
    let live = true;
    const t = setTimeout(() => {
      api
        .validate(text)
        .then((v) => live && setOwners(v.ok && v.owners ? v.owners : NONE))
        .catch(() => live && setOwners(NONE));
    }, SETTLE_MS);
    return () => {
      live = false;
      clearTimeout(t);
    };
  }, [text, enabled]);
  return enabled ? owners : NONE;
}
