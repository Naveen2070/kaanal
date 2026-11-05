//! Shared helpers: topology, hops, plans and the run loop.

use kaanal_core::replay::{Bucket, Engine, Finished};

use kaanal_core::topology::RateLimit;

use kaanal_core::{ChainResult, Config, Hop, HopKind, NodeId, Plan, PlanItem, Topology};

use std::sync::Arc;

use std::time::Duration;

pub(crate) const MS: u64 = 1_000_000;

pub(crate) fn bucket(interval_ms: u64, burst: u32) -> Bucket {
    Bucket::new(&RateLimit {
        interval_ns: interval_ms * MS,
        burst,
    })
}

pub(crate) fn topo(yaml: &str) -> Topology {
    Topology::build(&Config::from_yaml(yaml).unwrap()).unwrap()
}

pub(crate) fn hop(node: NodeId, kind: HopKind, ms: u64) -> PlanItem {
    PlanItem::Hop(Hop {
        node,
        replica: 0,
        kind,
        start: Duration::ZERO,
        dur: Duration::from_millis(ms),
    })
}

pub(crate) fn plan(items: Vec<PlanItem>) -> Arc<Plan> {
    Arc::new(Plan {
        items,
        ..Plan::default()
    })
}

/// Steps until nothing is scheduled and returns everything that finished.
pub(crate) fn run_all(e: &mut Engine) -> Vec<Finished> {
    while e.step().unwrap() {}
    e.drain_finished()
}

pub(crate) const ONE_SLOT: &str = "version: 1\nnodes:\n  app: { type: service, concurrency: 1 }\n";

pub(crate) fn ms(n: u64) -> u64 {
    n * MS
}

pub(crate) const LIMITED: &str = "version: 1\nnodes:\n  gw: { type: gateway, rate_limit: { rps: 1, burst: 1 }, concurrency: 1 }\n";

pub(crate) fn err_of(f: &Finished) -> (NodeId, u16, Option<Duration>) {
    match f.result {
        ChainResult::Err {
            node,
            status,
            kind: _,
            retry_after,
        } => (node, status, retry_after),
        ChainResult::Ok => panic!("expected a failure: {f:?}"),
    }
}

pub(crate) const POOL1: &str = "version: 1\nnodes:\n  gw: { type: service }\n  db: { type: database, ops: { read: 400ms, write: 400ms, scan: 400ms }, pool: { size: 1, queue: 5 } }\nedges:\n  - { from: gw, to: db, net: 1ms }\n";

pub(crate) fn span(node: NodeId, ms: u64, size: usize) -> PlanItem {
    PlanItem::Span {
        node,
        timeout: Duration::from_millis(ms),
        at: Duration::ZERO,
        size,
    }
}

pub(crate) fn fin(f: &[Finished], order: u64) -> &Finished {
    f.iter().find(|x| x.order == order).unwrap()
}
