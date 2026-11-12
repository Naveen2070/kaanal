import { expect, test } from "vite-plus/test";
import type { NodeDoc } from "../../data/api/types";
import {
  engineOf,
  enginesFor,
  hasManagedClasses,
  hostingsFor,
  isManagedClass,
  isManagedDb,
  isOwnClass,
  priceListKey,
  setEngine,
  setHosting,
  takesInstance,
} from "./hosting";

const node = (extra: Record<string, unknown>): NodeDoc => extra as NodeDoc;

test("each_kind_has_the_hostings_the_loader_allows_and_an_external_api_and_storage_have_none", () => {
  expect(hostingsFor("database")).toEqual(["self", "managed", "serverless", "external"]);
  expect(hostingsFor("service")).toEqual(["self", "serverless"]);
  expect(hostingsFor("gateway")).toEqual(["self", "managed"]);
  expect(hostingsFor("auth")).toEqual(["self", "external"]);
  expect(hostingsFor("external-api")).toEqual([]);
  expect(hostingsFor("storage")).toEqual([]);
});

test("a_managed_class_is_told_from_a_vm_type_and_only_self_and_a_managed_database_take_an_instance", () => {
  expect(isManagedClass("aws/db.m5.large")).toBe(true);
  expect(isManagedClass("aws/m5.large")).toBe(false);
  expect(takesInstance(node({ type: "database" }))).toBe(true);
  expect(isManagedDb(node({ type: "database", hosting: "managed" }))).toBe(true);
  expect(takesInstance(node({ type: "database", hosting: "managed" }))).toBe(true);
  expect(takesInstance(node({ type: "database", hosting: "serverless" }))).toBe(false);
  expect(takesInstance(node({ type: "gateway", hosting: "managed" }))).toBe(false);
});

test("choosing_a_hosting_writes_it_and_removes_what_no_longer_fits", () => {
  // self is the default: it is removed, not written
  expect(setHosting("d", node({ type: "database", hosting: "managed" }), "self")).toEqual([
    { op: "remove", path: ["nodes", "d", "hosting"] },
  ]);
  // a VM type does not go with a managed database: it is dropped with the change
  expect(setHosting("d", node({ type: "database", instance: "aws/m5.large" }), "managed")).toEqual([
    { op: "insert", path: ["nodes", "d", "hosting"], value: "managed" },
    { op: "remove", path: ["nodes", "d", "instance"] },
  ]);
  // your own instance stays on a managed database
  expect(setHosting("d", node({ type: "database", instance: "rack" }), "managed")).toEqual([
    { op: "insert", path: ["nodes", "d", "hosting"], value: "managed" },
  ]);
  // back to self drops a class and the engine
  expect(
    setHosting(
      "d",
      node({ type: "database", hosting: "managed", instance: "aws/db.m5.large", engine: "mysql" }),
      "self",
    ),
  ).toEqual([
    { op: "remove", path: ["nodes", "d", "hosting"] },
    { op: "remove", path: ["nodes", "d", "instance"] },
    { op: "remove", path: ["nodes", "d", "engine"] },
  ]);
  // serverless and external take no instance at all
  expect(setHosting("s", node({ type: "service", instance: "rack" }), "serverless")).toEqual([
    { op: "insert", path: ["nodes", "s", "hosting"], value: "serverless" },
    { op: "remove", path: ["nodes", "s", "instance"] },
  ]);
});

test("the_engine_default_is_not_written", () => {
  expect(setEngine("d", node({ type: "database" }), "postgresql")).toEqual([]);
  expect(setEngine("d", node({ type: "database", engine: "mysql" }), "postgresql")).toEqual([
    { op: "remove", path: ["nodes", "d", "engine"] },
  ]);
  expect(setEngine("d", node({ type: "database" }), "mariadb")).toEqual([
    { op: "insert", path: ["nodes", "d", "engine"], value: "mariadb" },
  ]);
});

test("a_cache_has_three_hostings_and_a_managed_cache_names_a_cache_class", () => {
  expect(hostingsFor("cache")).toEqual(["self", "managed", "serverless"]);
  expect(takesInstance(node({ type: "cache", hosting: "managed" }))).toBe(true);
  expect(takesInstance(node({ type: "cache", hosting: "serverless" }))).toBe(false);
  expect(isOwnClass("cache", "aws/cache.m5.large")).toBe(true);
  expect(isOwnClass("cache", "aws/db.m5.large")).toBe(false);
  expect(isOwnClass("database", "aws/db.m5.large")).toBe(true);
  expect(isManagedClass("aws/cache.m5.large")).toBe(true);
  // a database class does not stay on a cache that becomes managed
  expect(setHosting("c", node({ type: "cache", instance: "aws/db.m5.large" }), "managed")).toEqual([
    { op: "insert", path: ["nodes", "c", "hosting"], value: "managed" },
    { op: "remove", path: ["nodes", "c", "instance"] },
  ]);
});

test("the_price_list_follows_the_hosting_the_kind_and_the_engine", () => {
  expect(priceListKey(node({ type: "service" }), "aws")).toBe("aws");
  expect(priceListKey(node({ type: "cache", hosting: "managed" }), "aws")).toBe("aws-elasticache");
  expect(priceListKey(node({ type: "cache", hosting: "managed", flavor: "redis" }), "aws")).toBe(
    "aws-elasticache",
  );
  expect(priceListKey(node({ type: "cache", hosting: "managed", flavor: "valkey" }), "aws")).toBe(
    "aws-elasticache-valkey",
  );
  expect(priceListKey(node({ type: "database", hosting: "managed" }), "aws")).toBe(
    "aws-rds-postgresql",
  );
  expect(priceListKey(node({ type: "database", hosting: "managed", flavor: "mysql" }), "aws")).toBe(
    "aws-rds-mysql",
  );
  expect(
    priceListKey(node({ type: "database", hosting: "managed", engine: "mariadb" }), "aws"),
  ).toBe("aws-rds-mariadb");
  expect(priceListKey(node({ type: "service", hosting: "serverless" }), "aws")).toBeNull();
});

test("the_engines_follow_the_flavor_and_mongodb_neo4j_cassandra_dynamodb_have_none", () => {
  const ids = (flavor: string) => enginesFor(node({ type: "database", flavor })).map((e) => e.id);
  expect(ids("postgres")).toEqual(["postgresql", "aurora-postgresql"]);
  expect(ids("mysql")).toEqual(["mysql", "aurora-mysql"]);
  expect(ids("mariadb")).toEqual(["mariadb"]);
  for (const f of ["mongodb", "neo4j", "cassandra", "dynamodb"]) {
    expect(ids(f)).toEqual([]);
    expect(hasManagedClasses(node({ type: "database", flavor: f }))).toBe(false);
  }
  // no flavor written: every engine, PostgreSQL first
  expect(enginesFor(node({ type: "database" })).length).toBe(5);
  expect(engineOf(node({ type: "database", flavor: "mysql" }))).toBe("mysql");
  // the flavor's own default engine is not written
  expect(setEngine("d", node({ type: "database", flavor: "mysql" }), "mysql")).toEqual([]);
});
