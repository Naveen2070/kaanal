// The glossary: a modal dialog with the chapters as vertical tabs (Radix: arrow keys, roving
// tabindex) and a search box that looks across all of them. Esc or Close return focus to the book
// button. A separate chunk, with its words (content.ts) and the shortcut list.

import * as TabsPrimitive from "@radix-ui/react-tabs";
import { X } from "lucide-react";
import { useLayoutEffect, useRef, useState } from "react";
import { SHORTCUTS } from "../palette/Shortcuts";
import { CHAPTERS, searchEntries, type Chapter, type Entry } from "./content";
import { SampleView } from "./Samples";
import { useGuide } from "./store";

/** The keyboard chapter is built from the same list as the shortcuts sheet, so they never differ. */
const KEYS: Chapter = {
  id: "keys",
  title: "Keys",
  intro: "Press ? anywhere on the canvas for this list in a small sheet.",
  entries: SHORTCUTS.map(([keys, what]) => ({ id: keys, term: keys, text: what })),
};
const ALL = [...CHAPTERS, KEYS];

function EntryRow({ entry, chapter }: { entry: Entry; chapter?: string }) {
  return (
    <div
      className={`border-b border-rule py-3 last:border-b-0 ${
        entry.sample ? "sm:grid sm:grid-cols-[minmax(0,1fr)_15rem] sm:gap-5" : ""
      }`}
    >
      <div className="min-w-0">
        <dt className="text-sm font-semibold text-text">
          {entry.term}
          {chapter ? <span className="ml-2 text-xs font-normal text-muted">{chapter}</span> : null}
        </dt>
        <dd className="m-0 mt-1 text-sm text-text">{entry.text}</dd>
        {entry.real ? (
          <dd className="m-0 mt-1 text-xs text-muted">In a real system: {entry.real}</dd>
        ) : null}
      </div>
      {entry.sample ? (
        <dd className="m-0 mt-3 sm:mt-0">
          <SampleView sample={entry.sample} />
        </dd>
      ) : null}
    </div>
  );
}

export function Glossary() {
  const box = useRef<HTMLDialogElement>(null);
  const chapter = useGuide((s) => s.chapter);
  const setChapter = useGuide((s) => s.setChapter);
  const hide = useGuide((s) => s.hide);
  const [text, setText] = useState("");
  const found = searchEntries(text, ALL);

  useLayoutEffect(() => {
    const d = box.current;
    if (!d) return;
    const opener = document.activeElement as HTMLElement | null;
    if (!d.open) {
      if (typeof d.showModal === "function") d.showModal();
      else d.setAttribute("open", "");
    }
    return () => opener?.focus();
  }, []);

  return (
    <dialog
      ref={box}
      aria-label="Glossary"
      aria-modal="true"
      onCancel={(e) => {
        e.preventDefault();
        hide();
      }}
      className="fixed top-1/2 left-1/2 m-0 flex h-[min(40rem,calc(100vh-3rem))] w-[min(56rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-2xl border border-border bg-sheet p-0 text-text shadow-float backdrop:bg-black/40"
    >
      <div className="flex items-center gap-3 border-b border-border px-4 py-3">
        <h2 className="m-0 text-base font-semibold">Glossary</h2>
        <input
          name="glossary-search"
          type="search"
          aria-label="Search the glossary"
          placeholder="Search every chapter"
          value={text}
          onChange={(e) => setText(e.currentTarget.value)}
          className="ml-auto h-8 w-64 max-w-full rounded-md border border-control bg-sheet px-2 text-sm"
        />
        <button
          type="button"
          aria-label="Close the glossary"
          onClick={hide}
          className="flex size-8 items-center justify-center rounded-md text-muted hover:bg-raised hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
        >
          <X size={16} aria-hidden />
        </button>
      </div>
      {text.trim() ? (
        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-2">
          <p aria-live="polite" className="m-0 py-2 text-xs text-muted">
            {found.length === 0
              ? "Nothing matches."
              : `${found.length} ${found.length === 1 ? "entry" : "entries"}`}
          </p>
          <dl className="m-0">
            {found.map(({ chapter: c, entry }) => (
              <EntryRow key={`${c.id}-${entry.id}`} entry={entry} chapter={c.title} />
            ))}
          </dl>
        </div>
      ) : (
        <TabsPrimitive.Root
          orientation="vertical"
          value={chapter}
          onValueChange={setChapter}
          className="flex min-h-0 flex-1"
        >
          <TabsPrimitive.List
            aria-label="Chapters"
            className="flex w-44 shrink-0 flex-col gap-0.5 border-r border-border p-2"
          >
            {ALL.map((c) => (
              <TabsPrimitive.Trigger
                key={c.id}
                value={c.id}
                className="rounded-md px-3 py-2 text-left text-sm text-muted hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent data-[state=active]:bg-active data-[state=active]:font-medium data-[state=active]:text-text"
              >
                {c.title}
              </TabsPrimitive.Trigger>
            ))}
          </TabsPrimitive.List>
          {ALL.map((c) => (
            <TabsPrimitive.Content
              key={c.id}
              value={c.id}
              className="min-h-0 flex-1 overflow-y-auto px-5 py-4 outline-none"
            >
              <h3 className="m-0 text-base font-semibold">{c.title}</h3>
              <p className="m-0 mt-1 max-w-prose text-sm text-muted">{c.intro}</p>
              <dl className="m-0 mt-2">
                {c.entries.map((entry) => (
                  <EntryRow key={entry.id} entry={entry} />
                ))}
              </dl>
            </TabsPrimitive.Content>
          ))}
        </TabsPrimitive.Root>
      )}
    </dialog>
  );
}
