// "Edit response" for one route, wherever a route is listed in Admin mode (the traffic mix, each row
// of the traffic builder): a button that opens the response editor on that route and says when the
// route already answers an edit. The edit is the route's sample payload: real requests get it, and a
// generated request of that route whose profile leaves the response size out is as large as it
// (RULES 117). Admin only; other modes show nothing.

import { useCallback, useEffect, useState } from "react";
import { listPayloads } from "../../data/api/payload";
import { useMode } from "../../data/store/mode";
import { LazyBoundary } from "../../kit/LazyBoundary";
import { PayloadEditor } from "./PayloadEditor.lazy";

/** The routes that answer a live edit, read when asked for and again after the editor closes. */
export function useEditedRoutes(active: boolean): {
  edited: ReadonlySet<string>;
  reload: () => void;
} {
  const [edited, setEdited] = useState<ReadonlySet<string>>(new Set());
  const reload = useCallback(() => {
    if (!active) return;
    listPayloads().then(
      (list) => setEdited(new Set(list.map((e) => e.route))),
      () => setEdited(new Set()),
    );
  }, [active]);
  useEffect(reload, [reload]);
  return { edited, reload };
}

/** The route key the editor takes: a HEAD is edited through its GET. */
const editable = (route: string): string => route.replace(/^HEAD /, "GET ");

export function EditResponse({
  route,
  edited = false,
  onClose,
}: {
  /** `GET /users/{id}`, as the traffic mix names it. */
  route: string;
  edited?: boolean;
  onClose?: () => void;
}) {
  const admin = useMode((s) => s.mode === "admin");
  const [open, setOpen] = useState(false);
  if (!admin) return null;
  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        title="Edit what this route answers. Its size sets the response size of generated traffic when the profile leaves it out."
        aria-label={`Edit response of ${route}`}
        className="h-7 shrink-0 rounded-md border border-control bg-sheet px-2 text-xs whitespace-nowrap hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
      >
        {edited ? "Edit · edited" : "Edit"}
      </button>
      {open ? (
        <LazyBoundary fallback={null}>
          <PayloadEditor
            route={editable(route)}
            onClose={() => {
              setOpen(false);
              onClose?.();
            }}
          />
        </LazyBoundary>
      ) : null}
    </>
  );
}
