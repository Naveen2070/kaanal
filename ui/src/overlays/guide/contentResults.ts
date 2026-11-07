// The chapter "Reading Results", built from the help copy (help/copyResults.ts): the help button on a
// chart title and its glossary entry are the same words. The entry's id is the help id.

import { RESULTS_HELP } from "../help/copyResults";
import type { Chapter } from "./content";

export const RESULTS_CHAPTER: Chapter = {
  id: "results",
  title: "Reading Results",
  intro:
    "What each chart on the Results tab shows and how to read it. Each has a help button beside its title that opens its entry here.",
  entries: Object.entries(RESULTS_HELP).map(([id, h]) => ({
    id,
    term: h.label,
    text: h.more ? `${h.what} ${h.more}` : h.what,
    real: h.real,
    example: h.example,
    hint: h.hint,
  })),
};
