// The chapter "Wire settings", built from the help copy (help/copyWire.ts): the help button on a
// control and its glossary entry are the same words, so they cannot differ. The entry's id is the
// help id, which is what the help button's link opens.

import { WIRE_HELP } from "../help/copyWire";
import { WHERE } from "../help/where";
import type { Chapter } from "./content";

export const SETTINGS_CHAPTER: Chapter = {
  id: "settings",
  title: "Wire settings",
  intro:
    "What each setting on a wire, a retry, a breaker or a store does, with a worked example. Each has a help button beside it on the screen that opens its entry here.",
  entries: Object.entries(WIRE_HELP).map(([id, h]) => ({
    id,
    term: h.label,
    text: h.more ? `${h.what} ${h.more}` : h.what,
    real: h.real,
    example: h.example,
    hint: h.hint,
    where: (WHERE as Record<string, string | undefined>)[id],
  })),
};
