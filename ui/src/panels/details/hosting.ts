// Where a component runs (`hosting:`, RULES 122) as data: which hostings each kind has, the words
// Studio shows, and the patch that changes it. The rules mirror the loader's (`config/hosting.rs`):
// only `self` takes a VM type, only a `managed` database takes a managed class (`aws/db.m5.large`),
// the other hostings take no instance, and `engine` goes with a managed database only.

import type { NodeDoc, Op } from "../../data/api/types";
import { put } from "./fields.base";

export type HostingId = "self" | "managed" | "serverless" | "external";

/** The hostings of each kind, the default first; a kind not listed has none to choose. */
const BY_KIND: Readonly<Record<string, readonly HostingId[]>> = {
  gateway: ["self", "managed"],
  "load-balancer": ["self", "managed"],
  service: ["self", "serverless"],
  monolith: ["self", "serverless"],
  database: ["self", "managed", "serverless", "external"],
  cache: ["self", "managed", "serverless"],
  auth: ["self", "external"],
};

export const hostingsFor = (type: string): readonly HostingId[] => BY_KIND[type] ?? [];

/** What the option says, with the example that fits the kind. */
export function hostingLabel(type: string, h: HostingId): string {
  switch (h) {
    case "self":
      return (
        {
          gateway: "Self-hosted (nginx or Envoy on a machine you run)",
          "load-balancer": "Self-hosted (HAProxy or nginx on a machine you run)",
          service: "Self-hosted (your VM or container host)",
          monolith: "Self-hosted (your VM or container host)",
          database: "Self-hosted (the database on a machine you run)",
          cache: "Self-hosted (Redis, Valkey or Memcached on a machine you run)",
          auth: "Self-hosted (Keycloak or similar on a machine you run)",
        }[type] ?? "Self-hosted (a machine you run)"
      );
    case "managed":
      return (
        {
          gateway: "Managed gateway (API Gateway, Cloud Endpoints)",
          "load-balancer": "Managed load balancer (ALB, Cloud Load Balancing)",
          database: "Managed database (RDS-style class)",
          cache: "Managed cache (ElastiCache, Memorystore)",
        }[type] ?? "Managed by the provider"
      );
    case "serverless":
      return (
        {
          service: "Serverless (Lambda, Cloud Run, Fargate)",
          monolith: "Serverless (Lambda, Cloud Run, Fargate)",
          database: "Serverless (DynamoDB, Aurora Serverless)",
          cache: "Serverless cache (ElastiCache Serverless, Upstash)",
        }[type] ?? "Serverless (pay per use)"
      );
    case "external":
      return type === "auth"
        ? "Hosted identity provider (Auth0, Okta, Cognito)"
        : "Run by someone else";
  }
}

/** One sentence under the select: what the hosting does to the instance and the cost estimate. */
export function hostingHint(type: string, h: HostingId): string {
  switch (h) {
    case "self":
      return type === "cache"
        ? "Redis, Valkey or Memcached on a VM you run, in any cloud. A VM type is priced as a VM and shown as a size note: a cache's hits are fixed times, so its cores do not change them."
        : "A machine you run in any cloud or on premises, nginx on a VM included. A VM type fills the cores, memory and network card, and the cost estimate prices it as a VM.";
    case "managed":
      if (type === "database") {
        return "Pick a managed class (AWS RDS): it fills the cores, memory and network card, and the estimate prices it per engine. Azure and Google Cloud classes are not in the table yet: set cores and memory yourself.";
      }
      if (type === "cache") {
        return "Pick a managed class (AWS ElastiCache) for its price and size; the valkey flavor is priced as ElastiCache for Valkey, 20% below Redis. Azure and Google Cloud classes are not in the table yet. A cache's hits are fixed times, so the class does not change them.";
      }
      return "The provider runs and scales it, so there is no instance to pick. Set cores and memory yourself if you model them. The estimate leaves it out: it is priced by use.";
    case "serverless":
      return "Pay per use, so there is no instance to pick. Set cores and memory yourself if you model them. The estimate leaves it out.";
    case "external":
      return "Someone else runs it, so only its time and limits are yours. The estimate leaves it out.";
  }
}

export const ENGINES: readonly { id: string; label: string }[] = [
  { id: "postgresql", label: "PostgreSQL" },
  { id: "mysql", label: "MySQL" },
  { id: "mariadb", label: "MariaDB" },
  { id: "aurora-postgresql", label: "Aurora PostgreSQL" },
  { id: "aurora-mysql", label: "Aurora MySQL" },
];

export function hostingOf(config: NodeDoc): HostingId {
  const h = config.hosting;
  return h === "managed" || h === "serverless" || h === "external" ? h : "self";
}

/** The database's `flavor:` (postgres, mysql, mongodb, ...), "" when none. */
export const flavorOf = (config: NodeDoc): string =>
  typeof config.flavor === "string" ? config.flavor : "";

/** The managed engines that run a database's flavor, the first its default; every engine when no flavor is written, none for a flavor with no class in the table (MongoDB, Neo4j, Cassandra, DynamoDB). */
export function enginesFor(config: NodeDoc): readonly { id: string; label: string }[] {
  const ids: readonly string[] | null =
    {
      postgres: ["postgresql", "aurora-postgresql"],
      mysql: ["mysql", "aurora-mysql"],
      mariadb: ["mariadb"],
      mongodb: [],
      neo4j: [],
      cassandra: [],
      dynamodb: [],
    }[flavorOf(config)] ?? null;
  return ids === null ? ENGINES : ENGINES.filter((e) => ids.includes(e.id));
}

export function engineOf(config: NodeDoc): string {
  if (typeof config.engine === "string" && config.engine !== "") return config.engine;
  return enginesFor(config)[0]?.id ?? "postgresql";
}

/** The engines a managed cache runs; each is a cache `flavor`, and the price of a class depends on it. */
export const CACHE_ENGINES = [
  { id: "redis", label: "Redis OSS" },
  { id: "valkey", label: "Valkey (20% cheaper)" },
  { id: "memcached", label: "Memcached" },
] as const;

/** The cache's engine: its `flavor:`, Redis when none is written. */
export const cacheEngineOf = (config: NodeDoc): string => flavorOf(config) || "redis";

/** Whether the node has a Multi-AZ switch: a managed database that is not Aurora (Aurora keeps copies with `replicas`). */
export const hasMultiAz = (config: NodeDoc): boolean =>
  isManagedDb(config) && hasManagedClasses(config) && !engineOf(config).startsWith("aurora");

/** The patch for the Multi-AZ switch: on writes `multi_az: true`, off removes it (the default). */
export function setMultiAz(name: string, config: NodeDoc, on: boolean): Op[] {
  const path = ["nodes", name, "multi_az"];
  const present = "multi_az" in config;
  if (!on) return present ? [{ op: "remove", path }] : [];
  return [{ op: present ? "set" : "insert", path, value: true }];
}

/** Whether the table has managed classes for this database (the relational flavors; not MongoDB, Neo4j, Cassandra or DynamoDB). */
export const hasManagedClasses = (config: NodeDoc): boolean =>
  config.type !== "database" || enginesFor(config).length > 0;

/** Whether the node is a database with a managed class (so it has an engine). */
export const isManagedDb = (config: NodeDoc): boolean =>
  config.type === "database" && hostingOf(config) === "managed";

/** Whether the node is a database or a cache a provider runs: it names a managed class. */
export const isManagedService = (config: NodeDoc): boolean =>
  (config.type === "database" || config.type === "cache") && hostingOf(config) === "managed";

/** Whether a node of this hosting has an instance to pick at all. */
export const takesInstance = (config: NodeDoc): boolean =>
  hostingOf(config) === "self" || isManagedService(config);

/** Whether `instance` names a managed class (`db.` or `cache.`), not a VM type. */
export const isManagedClass = (instance: string): boolean =>
  /^[^/]+\/(db|cache)\./.test(instance.trim().toLowerCase());

/** Whether `instance` is the managed class of the kind's own service (`db.` for a database, `cache.` for a cache). */
export const isOwnClass = (type: unknown, instance: string): boolean =>
  new RegExp(`^[^/]+/${type === "cache" ? "cache" : "db"}\\.`).test(instance.trim().toLowerCase());

/**
 * The price list a node's instance is read from, in the region of `provider`: a managed database
 * class per engine (`aws-rds-mysql`), a managed cache class per engine (`aws-elasticache`, or
 * `aws-elasticache-valkey` for the valkey flavor, 20% cheaper), else the
 * provider's own list; null when the hosting has no instance to price.
 */
export function priceListKey(
  config: NodeDoc | Readonly<Record<string, unknown>>,
  provider: string,
): string | null {
  const h = (config as NodeDoc).hosting;
  const hosting = h === "managed" || h === "serverless" || h === "external" ? h : "self";
  if (hosting === "serverless" || hosting === "external") return null;
  if (hosting === "managed") {
    if (config.type === "cache") {
      return `${provider}-elasticache${flavorOf(config as NodeDoc) === "valkey" ? "-valkey" : ""}`;
    }
    if (config.type === "database") return `${provider}-rds-${engineOf(config as NodeDoc)}`;
    return null;
  }
  return provider;
}

/** Whether the `instance` the node names fits the hosting (the loader refuses the rest). */
function instanceFits(config: NodeDoc, next: HostingId): boolean {
  const i = typeof config.instance === "string" ? config.instance.trim() : "";
  if (i === "") return true;
  const custom = !i.includes("/");
  if (next === "self") return !isManagedClass(i);
  if (next === "managed" && (config.type === "database" || config.type === "cache")) {
    return custom || isOwnClass(config.type, i);
  }
  return false;
}

/**
 * The patch for choosing a hosting: writes `hosting` (removed for `self`, the default) and removes
 * what no longer fits: an `instance` of the other catalogue, and an `engine` off a managed database.
 */
export function setHosting(name: string, config: NodeDoc, next: HostingId): Op[] {
  const ops: Op[] = [];
  const base = ["nodes", name];
  const present = "hosting" in config;
  if (next === "self") {
    if (present) ops.push({ op: "remove", path: [...base, "hosting"] });
  } else ops.push(put([...base, "hosting"], present, next));
  if ("instance" in config && !instanceFits(config, next)) {
    ops.push({ op: "remove", path: [...base, "instance"] });
  }
  if ("engine" in config && !(config.type === "database" && next === "managed")) {
    ops.push({ op: "remove", path: [...base, "engine"] });
  }
  return ops;
}

/** The patch for the engine of a managed database (PostgreSQL, the default, removes the key). */
export function setEngine(name: string, config: NodeDoc, id: string): Op[] {
  const path = ["nodes", name, "engine"];
  const present = "engine" in config;
  // the default of the flavor is not written
  if (id === (enginesFor(config)[0]?.id ?? "postgresql"))
    return present ? [{ op: "remove", path }] : [];
  return [put(path, present, id)];
}
