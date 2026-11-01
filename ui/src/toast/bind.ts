// Turns the state the stores already carry into toasts: a run that could not give numbers, a refused
// or failed edit, a change the history cannot take back. Each source has a key, so its message
// replaces its older one and its recovery closes it. Returns the unsubscribe.

import { useDraft } from "../store/draft";
import { useHistory } from "../store/history";
import { useRun } from "../store/run";
import { notify } from "./toasts";

export function startNotifications(): () => void {
  const offRun = useRun.subscribe((s, prev) => {
    if (s.lastError === prev.lastError) return;
    if (s.lastError) notify.error(s.lastError, "run");
    else notify.clear("run");
  });
  const offDraft = useDraft.subscribe((s, prev) => {
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
