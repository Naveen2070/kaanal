// The glossary's words (a lazy chunk with its dialog). Plain data, so a test can check it and the
// dialog only renders it. Every statement here is what the simulator and the loader really do
// (rules R1.24, the chain engine's default chain, the card and key-figure definitions); a new
// figure on the screen needs its entry here. Examples of real products say what a component
// stands for, never what Kaanal measures. A `sample` is a small picture of the thing, drawn with
// the real icons, hues and meter (Samples.tsx); its figures are examples, and the dialog says so.

/** One step of a chain sample: a component, and what happens at it. */
import { ADMIN_CHAPTER } from "./contentAdmin";
import { RESULTS_CHAPTER } from "./contentResults";
import { SETTINGS_CHAPTER } from "./contentSettings";

export interface ChainStep {
  of: string;
  /** A few words under the tile ("asks", "then calls"). */
  note?: string;
  /** The step failed: marked in red. */
  failed?: boolean;
  /** The step never happens because an earlier one failed: faded. */
  skipped?: boolean;
}

/** A group of slots with its own queue, for the slots sample. */
export interface SlotGroup {
  slots: number;
  busy: number;
  waiting: number;
  /** The queue limit, drawn as that many places; omitted for no limit. */
  limit?: number;
}

export type Sample =
  /** A component's tile: its icon in its hue, and its name. */
  | { kind: "tile"; of: string }
  /** A whole card, drawn as on the canvas. */
  | {
      kind: "card";
      of: string;
      name: string;
      line: string;
      ratio: number;
      sentence: string;
      rows: [string, string][];
    }
  /** Meters at different levels, each with its words. */
  | { kind: "meters"; items: { ratio: number; label: string; note?: string }[] }
  /** Slots as squares, filled when busy, with the queue as dots. One group per copy. */
  | { kind: "slots"; groups: SlotGroup[] }
  /** A wire between two tiles; idle is the dashed wire that carried nothing. */
  | { kind: "wire"; from: string; to: string; idle?: boolean }
  /** A key figure: its label over its value. */
  | { kind: "figure"; label: string; value: string; note?: string; slider?: number }
  /** The draft state line, in both of its moods. */
  | { kind: "draft" }
  /** A request's time at a component: waiting then work. */
  | { kind: "time"; wait: number; work: number }
  /** Components in the order a request meets them. */
  | { kind: "chain"; steps: ChainStep[] }
  /** Who may call whom: each caller with the kinds it may call. */
  | { kind: "calls"; rows: { from: string; to: string[] }[] };

export interface Entry {
  id: string;
  term: string;
  /** What it means in Kaanal, in plain words. */
  text: string;
  /** What it stands for in a real system, when it stands for something. */
  real?: string;
  /** A worked example, for the settings. */
  example?: string;
  /** When to change it, for the settings. */
  hint?: string;
  /** Where its control is on screen, for the settings that have one. */
  where?: string;
  sample?: Sample;
}

export interface Chapter {
  id: string;
  title: string;
  intro: string;
  entries: Entry[];
}

export const CHAPTERS: Chapter[] = [
  {
    id: "components",
    title: "Components",
    intro:
      "Each box on the canvas stands for one part of a real system. Kaanal does not run your code: it asks how many requests the part can work on at once, how long one takes, and how many may wait.",
    entries: [
      {
        id: "gateway",
        term: "Gateway",
        text: "The front door: where requests enter. It does its own work first, then asks each auth component it is wired to, then passes the request on to the service that owns the route.",
        real: "nginx, Envoy, Kong, an API gateway or a load balancer in front of your apps.",
        sample: { kind: "tile", of: "gateway" },
      },
      {
        id: "service",
        term: "Service",
        text: "An app that does work and calls others. It does its own work, then the services it calls, then its cache for reads, then its databases.",
        real: "A Node, Spring, Go or Rails server. Slots stand for the requests it handles at once (threads or workers); copies stand for instances of it (replicas or pods).",
        sample: { kind: "tile", of: "service" },
      },
      {
        id: "monolith",
        term: "Monolith",
        text: "One big app that does the work of several services. It can carry its own read, write and scan costs, as well as calling a database or cache.",
        real: "A Rails, Django, Laravel or Spring Boot application that serves everything.",
        sample: { kind: "tile", of: "monolith" },
      },
      {
        id: "database",
        term: "Database",
        text: "Stores data. Its slots are the connection pool: only that many reads or writes run at once. Reads, writes and scans each have their own time.",
        real: "Postgres, MySQL or MongoDB, reached through a pool of connections.",
        sample: { kind: "tile", of: "database" },
      },
      {
        id: "cache",
        term: "Cache",
        text: "Answers repeat reads fast. A hit ends the work and skips the database; a miss carries on to the database. Writes skip it. A cache that is down counts as a miss, so the database serves the read, slower.",
        real: "Redis or Memcached in front of a database.",
        sample: { kind: "tile", of: "cache" },
      },
      {
        id: "external-api",
        term: "External API",
        text: "Someone else's service that you call. It has its own time and you cannot change how it is built.",
        real: "A payment, email, SMS or maps provider.",
        sample: { kind: "tile", of: "external-api" },
      },
      {
        id: "auth",
        term: "Auth",
        text: "Checks who is calling. The gateway's token check charges the auth component's own time only. The login route (POST /login) also follows the wires that leave auth, for example to a user database or a mail service.",
        real: "Keycloak, Auth0 or Cognito, or an nginx auth_request pointed at an auth service.",
        sample: { kind: "tile", of: "auth" },
      },
      {
        id: "load-balancer",
        term: "Load balancer",
        text: "A hop that sends each call to one of the components it has wires to, round robin, at random or by weight. It has its own time, slots and bandwidth, and you can have several (for example one in front of each tier). Unlike the gateway it is never where requests enter. A component with no live copy is skipped; with none left the call fails with a 503.",
        real: "nginx, HAProxy, Envoy or a cloud load balancer in front of a tier.",
        sample: { kind: "tile", of: "load-balancer" },
      },
      {
        id: "storage",
        term: "Storage",
        text: "An object store. Each call costs a first byte, then the size of the request and the reply over the throughput. The numbers are yours: no provider publishes a first-byte latency, so a new Storage node starts with placeholders, not published figures. The flavor (S3, GCS or Blob) is only a label.",
        real: "Amazon S3, Google Cloud Storage or Azure Blob Storage.",
        sample: { kind: "tile", of: "storage" },
      },
    ],
  },
  {
    id: "screen",
    title: "Reading the screen",
    intro:
      "Every number comes from the newest simulated run of your draft. The run is a relative what-if, not a prediction: use it to see which part gives out first and by how much, not to promise a real latency.",
    entries: [
      {
        id: "card-line",
        term: "Service · 1 copy × 32 slots · 25 ms",
        text: "The card's own settings, in words: what kind it is, how many copies, how many slots each, and how long one request takes once it has a slot. 'Time varies' means the time is a spread of values set in the file.",
        sample: {
          kind: "card",
          of: "service",
          name: "shop",
          line: "Service · 1 copy × 32 slots · 25 ms",
          ratio: 0.83,
          sentence: "83% busy · up to 21 waiting",
          rows: [
            ["Arriving", "1,062/s"],
            ["Slowest work under", "32 ms"],
          ],
        },
      },
      {
        id: "busy",
        term: "83% busy",
        text: "The share of the component's slots in use over the run. For a thread per request (see Server style) a slot counts as in use while the component waits on what it calls, so a slow dependency makes it look busy. The bar is green below 70%, amber from 70% and red from 90%; the words say the same, so colour is never the only signal.",
        sample: {
          kind: "meters",
          items: [
            { ratio: 0.45, label: "45% busy · nothing waiting", note: "below 70%" },
            { ratio: 0.83, label: "83% busy · up to 21 waiting", note: "from 70%" },
            { ratio: 0.95, label: "95% busy · up to 60 waiting", note: "from 90%" },
          ],
        },
      },
      {
        id: "waiting",
        term: "up to 21 waiting",
        text: "The longest queue seen during the run. It is a peak, not a live count. Some waiting is normal well below 100% busy, because requests arrive in bursts.",
        sample: { kind: "slots", groups: [{ slots: 8, busy: 7, waiting: 3 }] },
      },
      {
        id: "saturated",
        term: "Saturated",
        text: "No spare capacity left: every slot is full, so new requests queue up and then get turned away.",
        sample: {
          kind: "meters",
          items: [{ ratio: 1, label: "Saturated · up to 40 waiting" }],
        },
      },
      {
        id: "server-style",
        term: "Server style",
        text: "How a component spends its slots. An event loop (the default) holds a slot only while it works; while it waits on something it calls, the slot is free. A thread per request holds its slot until everything it calls has answered, so a slow database fills the app above it and the app shows busy, queues and turns requests away. Choosing a thread per request on a component with no slot limit gives it 200 slots and a queue of 100; change them in Capacity.",
        real: "Event loop: Node, Go, async Python. Thread per request: Tomcat, sync Gunicorn, Puma, PHP-FPM.",
      },
      {
        id: "turning-away",
        term: "Turning requests away",
        text: "The component refused requests because its queue was full or a rate limit applied. It can be only partly busy and still turn requests away, for example with a queue limit of 0, where a request is refused the moment every slot is taken.",
        sample: {
          kind: "card",
          of: "database",
          name: "db",
          line: "Database · 1 copy × 1 slot",
          ratio: 0.45,
          sentence: "45% busy · turning requests away",
          rows: [
            ["Arriving", "102/s"],
            ["Turned away", "64/s"],
          ],
        },
      },
      {
        id: "arriving",
        term: "Arriving",
        text: "Requests per second that got in and were worked on here. Requests turned away are not counted, which is why a component can show less than the one calling it.",
        sample: {
          kind: "card",
          of: "gateway",
          name: "nginx",
          line: "Gateway · 1 copy × 256 slots · 2 ms",
          ratio: 0.01,
          sentence: "1% busy · nothing waiting",
          rows: [["Arriving", "301/s"]],
        },
      },
      {
        id: "turned-away",
        term: "Turned away",
        text: "Shown only when it is not zero: how many requests per second were refused here, scaled from the rate that got in.",
      },
      {
        id: "slowest",
        term: "Slowest work under",
        text: "The time within which 99 in every 100 requests finished their work here. It counts work only, not the time spent waiting in line or for a held slot, and it is exact over every request in the run.",
        sample: { kind: "time", wait: 12, work: 25 },
      },
      {
        id: "wire",
        term: "A wire",
        text: "Points from the caller to what it calls. The pulse moves toward the arrow, faster and heavier as the load grows. One lane per copy of the component it feeds. An amber or red glow under it means that component is running hot. A dashed wire carried nothing in the last run.",
        sample: { kind: "wire", from: "gateway", to: "service" },
      },
      {
        id: "breaks-first",
        term: "Breaks first",
        text: "The component that gives out at the lowest load, with the load (for a thread per request it falls when something it calls gets slower): 'shop at 2,100 req/s'. It turns red only when that component is already saturated at the current load; otherwise it reads 'nothing saturates'.",
        sample: { kind: "figure", label: "Breaks first", value: "shop at 2,100 req/s" },
      },
      {
        id: "offered-load",
        term: "Offered load",
        text: "How many requests per second you send in altogether. Drag the slider or type a number; every change re-runs the simulation.",
        sample: { kind: "figure", label: "Offered load", value: "300 /s", slider: 0.5 },
      },
      {
        id: "p99",
        term: "System p99",
        text: "End to end, 99 in every 100 requests finish faster than this. It includes time spent waiting, so it shows the slow tail that an average hides.",
        sample: { kind: "figure", label: "System p99", value: "36.5 ms" },
      },
      {
        id: "goodput",
        term: "Goodput",
        text: "Requests that finished successfully each second, not counting failed or turned-away ones.",
        sample: { kind: "figure", label: "Goodput", value: "301/s" },
      },
      {
        id: "errors",
        term: "Errors",
        text: "Requests that failed in the run: a component that was down, a failure you injected, or a caller that gave up at its timeout.",
        sample: { kind: "figure", label: "Errors", value: "0" },
      },
      {
        id: "dropped",
        term: "Dropped",
        text: "Requests turned away anywhere in the system, by a full queue or a rate limit, added together.",
        sample: { kind: "figure", label: "Dropped", value: "0" },
      },
      {
        id: "draft",
        term: "Draft, Apply, Save",
        text: "What you edit is a draft. Run only tries it. Apply makes the draft the running system, so the mocks switch to it. Save writes kaanal.yaml. 'Everything saved' means all three agree.",
        sample: { kind: "draft" },
      },
    ],
  },
  {
    id: "terms",
    title: "Terms",
    intro: "The words behind the settings. Each has a help button beside it on the screen.",
    entries: [
      {
        id: "time",
        term: "Time",
        text: "How long one request holds a slot while it is being worked on. Longer means fewer requests finished each second.",
        sample: { kind: "time", wait: 0, work: 25 },
      },
      {
        id: "slots",
        term: "Slots",
        text: "How many requests one copy works on at the same moment. More slots, more throughput, until the time per request limits it. Capacity is slots divided by time: 32 slots at 25 ms can finish about 1,280 a second.",
        sample: { kind: "slots", groups: [{ slots: 8, busy: 5, waiting: 0 }] },
      },
      {
        id: "copies",
        term: "Copies",
        text: "Identical copies of a component, each with its own slots and its own queue. Load is shared between them.",
        sample: {
          kind: "slots",
          groups: [
            { slots: 4, busy: 3, waiting: 1 },
            { slots: 4, busy: 2, waiting: 0 },
          ],
        },
      },
      {
        id: "queue",
        term: "Queue limit",
        text: "How many requests may wait for a free slot. Past it they are turned away. Empty means no limit; 0 means nobody waits, and a request is refused as soon as every slot is taken.",
        sample: { kind: "slots", groups: [{ slots: 4, busy: 4, waiting: 3, limit: 5 }] },
      },
      {
        id: "timeout",
        term: "Timeout",
        text: "A caller gives up on a component after this long, waiting and working included, and the request fails.",
      },
      {
        id: "saturation",
        term: "Saturation",
        text: "The point where a component has no spare capacity. Past it, requests queue up and then get turned away.",
        sample: { kind: "slots", groups: [{ slots: 6, busy: 6, waiting: 5 }] },
      },
      {
        id: "wait-work",
        term: "Waiting and work",
        text: "A request's time at a component is the time in the queue (waiting) plus the time in a slot (work). Slowest work under counts the second part only; System p99 counts both.",
        sample: { kind: "time", wait: 12, work: 25 },
      },
      {
        id: "chaos",
        term: "Chaos",
        text: "Break something on purpose: take a component down, slow it, or drop a share of requests. It acts on the running system, so Apply the draft first.",
      },
    ],
  },
  {
    id: "wiring",
    title: "Wires and routes",
    intro:
      "A wire is allowed by the kinds it joins (the loader checks the same table). Whether it carries traffic depends on the route a request takes.",
    entries: [
      {
        id: "who-calls-whom",
        term: "Who may call whom",
        text: "Gateway: service, monolith, auth, load balancer. Service and monolith: service, monolith, database, cache, external API, auth, load balancer, storage. Auth: service, monolith, database, cache, external API, load balancer, storage. Load balancer: service, monolith, database, external API, auth, load balancer, storage. Database, cache, external API and storage call nothing. There are no loops, and nothing calls the gateway.",
        sample: {
          kind: "calls",
          rows: [
            { from: "gateway", to: ["service", "monolith", "auth", "load-balancer"] },
            {
              from: "service",
              to: [
                "service",
                "monolith",
                "database",
                "cache",
                "external-api",
                "auth",
                "load-balancer",
                "storage",
              ],
            },
            {
              from: "auth",
              to: [
                "service",
                "monolith",
                "database",
                "cache",
                "external-api",
                "load-balancer",
                "storage",
              ],
            },
            {
              from: "load-balancer",
              to: [
                "service",
                "monolith",
                "database",
                "external-api",
                "auth",
                "load-balancer",
                "storage",
              ],
            },
          ],
        },
      },
      {
        id: "gateway-chain",
        term: "What the gateway does with a request",
        text: "Its own time, then each auth it is wired to, then a call into the service that owns the route. It must have a wire to that service, or requests stop at the gateway.",
        sample: {
          kind: "chain",
          steps: [
            { of: "gateway", note: "own work" },
            { of: "auth", note: "asks" },
            { of: "service", note: "then calls" },
          ],
        },
      },
      {
        id: "auth-check",
        term: "Auth is a check, not a forwarder",
        text: "For ordinary routes the gateway only asks auth and moves on itself; the request does not travel through auth. Wires leaving auth are used by the login route.",
        sample: {
          kind: "chain",
          steps: [
            { of: "gateway", note: "asks auth" },
            { of: "auth", note: "answers" },
            { of: "service", note: "called by the gateway" },
          ],
        },
      },
      {
        id: "failure-stops",
        term: "A failed step stops the request",
        text: "If auth fails or is down, the service is never called. The same holds for any step in a chain: the first failure ends it.",
        sample: {
          kind: "chain",
          steps: [
            { of: "gateway" },
            { of: "auth", note: "fails", failed: true },
            { of: "service", note: "never called", skipped: true },
          ],
        },
      },
      {
        id: "service-chain",
        term: "What a service does with a request",
        text: "Its own time, then the services it calls, then its cache for a read, then its databases. A cache hit ends the work early and skips the database.",
        sample: {
          kind: "chain",
          steps: [
            { of: "service", note: "own work" },
            { of: "service", note: "callee" },
            { of: "cache", note: "read" },
            { of: "database", note: "on a miss" },
          ],
        },
      },
      {
        id: "idle-wire",
        term: "No traffic in the last run",
        text: "A wire shown dashed, or marked so in the Settings tab: its source had requests and its target got none. Nothing is wrong with the wire; the route that traffic took did not use it.",
        sample: { kind: "wire", from: "auth", to: "service", idle: true },
      },
    ],
  },
  SETTINGS_CHAPTER,
  RESULTS_CHAPTER,
  ADMIN_CHAPTER,
];

/** Entries whose term, text or real-world note holds every word typed, with their chapter. */
export function searchEntries(
  text: string,
  chapters: Chapter[] = CHAPTERS,
): { chapter: Chapter; entry: Entry }[] {
  const words = text.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return [];
  return chapters.flatMap((chapter) =>
    chapter.entries
      .filter((entry) => {
        const hay = `${entry.term} ${entry.text} ${entry.real ?? ""}`.toLowerCase();
        return words.every((w) => hay.includes(w));
      })
      .map((entry) => ({ chapter, entry })),
  );
}
