// The chapter "Admin mode" (step 8A): a real frontend in the live numbers, and rewriting what a route
// answers. Plain words for the terms the Admin screen adds.

import type { Chapter } from "./content";

export const ADMIN_CHAPTER: Chapter = {
  id: "admin",
  title: "Admin mode",
  intro:
    "Admin mode is the live view with your own frontend in it. Point the frontend at the mock and its requests show in the numbers, next to the ones Kaanal generates. You can also rewrite what a route answers, while the frontend is running.",
  entries: [
    {
      id: "real-request",
      term: "Real request",
      text: "A request your frontend (or curl, or a test) sent to the mock. It runs through the same simulated system as the generated ones, so it waits in the same queues and meets the same chaos. On the canvas it is drawn as a ring.",
    },
    {
      id: "generated-request",
      term: "Generated request",
      text: "A request Kaanal makes up itself from the traffic profile, to keep the system busy while you work. On the canvas it is drawn as a dot. The Real only switch hides them from the figures.",
    },
    {
      id: "edited-response",
      term: "Edited response",
      text: "A status, headers and JSON body you chose for one route. From then on the route answers exactly that, with no Apply. It is kept in memory first (marked live only); Stage in the file writes it into the route's override, and Save writes the file. Chaos and pins still win: a route that is down or pinned to an error keeps failing, so you can test failures with your edit in place. A route that reads a collection or a login or key set built in to Kaanal takes no edit.",
      example:
        "Make GET /users/{id} answer 503 with a Retry-After header to see how your frontend backs off, then press Back to normal.",
    },
  ],
};
