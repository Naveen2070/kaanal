// Turns the state the stores already carry into toasts: a run that could not give numbers, a refused
// or failed edit, a change the history cannot take back. Each source has a key, so its message
// replaces its older one and its recovery closes it. Returns the unsubscribe.

import { useDraft } from "../../data/store/draft";
import { useHistory } from "../../data/store/history";
import { useRun } from "../../data/store/run";
import { notify } from "./toasts";

/** A blank draft cannot run, and saying so before anyone has drawn anything is noise, not news. */
const blank = () => Object.keys(useDraft.getState().view?.doc?.nodes ?? {}).length === 0;

export function startNotifications(): () => void {
  const offRun = useRun.subscribe((s, prev) => {
    if (s.lastError === prev.lastError) return;
    if (s.lastError && !blank()) notify.error(s.lastError, "run");
    else notify.clear("run");
  });
  const offDraft = useDraft.subscribe((s, prev) => {
    // the first component arrives while a run error is still standing: now it is news
    const err = useRun.getState().lastError;
    if (err && s.view?.doc?.nodes !== prev.view?.doc?.nodes && !blank()) notify.error(err, "run");
    if (s.error === prev.error) return;
    if (s.error) notify.error(s.error, "draft");
    else notify.clear("draft");
  });
  const offHistory = useHistory.subscribe((s, prev) => {
    if (s.note && s.note !== prev.note) notify.info(s.note, "history");
  });
  return () => {
    offRun();
    offDraft();
    offHistory();
  };
}
