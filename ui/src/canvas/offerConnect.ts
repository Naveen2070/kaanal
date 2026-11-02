// After a palette add with another component selected: a toast offering the wire from it (review
// C3). A lazy chunk (`useAddNode` imports it on first use) to keep the first load inside its budget.

import { useDraft } from "../store/draft";
import { notify } from "../toast/toasts";

export function offerConnect(from: string, name: string): void {
  notify.info(`Added ${name}`, "added", {
    label: `Connect ${from} to ${name}`,
    run: () => {
      notify.clear("connect");
      void useDraft
        .getState()
        .connect(from, name)
        .then((r) => {
          if (!r.ok) notify.error(r.message, "connect");
          else if (r.warning) notify.warn(r.warning, "connect");
        });
    },
  });
}
