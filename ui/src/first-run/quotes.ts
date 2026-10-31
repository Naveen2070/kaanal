// What the loading page says while Studio starts. Bundled, so there is no network and no wait for
// one. Written for this page (nothing quoted from anyone, so nothing is mis-attributed); no
// caption goes with them.

export const QUOTES: string[] = [
  // the mirage lines lead (spec 12); shown shuffled, never the same twice in a row
  "It looks like a backend. It answers like a backend. It is not one.",
  "Break the mirage here, before production does.",
  "A queue is a promise that someone will get to you eventually.",
  "Throughput is what you get; latency is what they feel.",
  "Somebody is always the p99.",
  "Saturation never announces itself. It just stops answering.",
  "Every hop adds a little wait. Enough hops add up to a page.",
  "Draw it here, break it here, ship it there.",
  "Every system is fast until the queue forms.",
  "The slowest dependency sets the pace for everyone who calls it.",
  "Capacity is a number; headroom is a decision.",
  "A retry is a request you asked for twice.",
  "Averages hide the afternoon the database fell over.",
  // echoes a well-known joke: kept until the developer says keep or replace (spec 15, open 2)
  "Add a cache and you have two problems: the old one, and when to forget.",
  "Load does not arrive evenly. Plan for the Monday morning.",
  "If everything is a priority, the queue decides.",
  "A timeout is a promise about how long you are willing to wait.",
  "The bottleneck moves the moment you fix it.",
  "Break it here first, where nobody is paged.",
  "A system under test tells you more than a system under glass.",
  "Slow is a failure with better manners.",
  "Know where the work waits and you know where to look.",
];
