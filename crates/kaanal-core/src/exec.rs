//! The chain executor: computes what a request would experience.
//!
//! Given a request, [`execute`] walks the topology path `gateway → service →
//! databases`, sampling distributions and applying chaos at each node. It is pure:
//! no sleeping, no IO. The live server sleeps the returned total; the simulator
//! advances virtual time (rule R2.5: one cost model, two drivers).

use std::time::Duration;

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::chaos::ChaosState;
use crate::dist::Dist;
use crate::stable_hash;
use crate::topology::{NodeId, Topology};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// A database operation. Discriminants index `Node::ops` ([read, write, scan]);
/// keep in sync with the topology.
pub enum Op {
    /// Read one row (`GET /users/{id}`).
    Read = 0,
    /// Insert, update or delete.
    Write = 1,
    /// List or search (`GET /users`).
    Scan = 2,
}

impl Op {
    /// GET with a path parameter reads one row; GET without lists; everything else writes.
    pub fn infer(method: &str, path_template: &str) -> Op {
        match method.to_ascii_uppercase().as_str() {
            "GET" | "HEAD" if path_template.contains('{') => Op::Read,
            "GET" | "HEAD" => Op::Scan,
            _ => Op::Write,
        }
    }

    /// Lowercase name used in the `X-Kaanal-Trace` header (`db.read=4.2ms`).
    pub fn name(self) -> &'static str {
        match self {
            Op::Read => "read",
            Op::Write => "write",
            Op::Scan => "scan",
        }
    }
}

/// What kind of cost a hop represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HopKind {
    /// Network latency on an edge.
    Net,
    /// Processing time on a gateway or service node.
    Proc,
    /// A database operation on a data node.
    Op(Op),
}

/// One step of a walked chain.
#[derive(Debug, Clone, PartialEq)]
pub struct Hop {
    /// The node the cost belongs to (for `Net` hops, the edge's target).
    pub node: NodeId,
    /// Net, proc or op.
    pub kind: HopKind,
    /// Sampled duration of this hop.
    pub dur: Duration,
}

/// How a chain walk ended.
#[derive(Debug, Clone, PartialEq)]
pub enum ChainResult {
    /// Every hop succeeded.
    Ok,
    /// The walk stopped at a node whose chaos failed it.
    Err {
        /// The failing node.
        node: NodeId,
        /// The HTTP status it answered with.
        status: u16,
    },
}

/// The full result of one [`execute`].
#[derive(Debug, Clone, PartialEq)]
pub struct ChainOutcome {
    /// Every hop walked, in order (hops before a failure still count).
    pub hops: Vec<Hop>,
    /// Ok, or where and how it failed.
    pub result: ChainResult,
}

impl ChainOutcome {
    /// Sum of all hop durations — what the client waits.
    pub fn total(&self) -> Duration {
        self.hops.iter().map(|h| h.dur).sum()
    }
}

/// The input to one [`execute`] call.
pub struct ChainRequest<'a> {
    /// Service that owns the route; `None` = static route answered by the gateway itself.
    pub entry: Option<NodeId>,
    /// The database operation this endpoint performs.
    pub op: Op,
    /// `"GET /users/{id}"` — part of the RNG seed, so each endpoint has its own stream.
    pub endpoint_key: &'a str,
    /// This endpoint's request counter — part of the seed, so sequential requests vary.
    pub n: u64,
    /// The config's master seed.
    pub seed: u64,
}

/// Computes what the request would experience.
///
/// Pure: the caller sleeps (live) or advances virtual time (sim). The RNG is seeded
/// from `stable_hash(seed, endpoint_key, n)`, so the same inputs always give the
/// same outcome (rule R4.1), and the chaos error roll is always drawn — enabling
/// chaos never shifts the timing stream.
pub fn execute(topo: &Topology, chaos: &ChaosState, req: &ChainRequest) -> ChainOutcome {
    let seed = stable_hash(&[
        &req.seed.to_le_bytes(),
        req.endpoint_key.as_bytes(),
        &req.n.to_le_bytes(),
    ]);
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut walk = Walk {
        topo,
        chaos,
        rng: &mut rng,
        hops: Vec::new(),
    };
    let result = walk.run(req);
    ChainOutcome {
        hops: walk.hops,
        result,
    }
}

/// **Private.** One walk in progress: borrowed inputs plus the hops collected so far.
struct Walk<'a> {
    topo: &'a Topology,
    chaos: &'a ChaosState,
    rng: &'a mut ChaCha8Rng,
    hops: Vec<Hop>,
}

impl Walk<'_> {
    /// **Private.** Gateway proc → net → service proc → (net → db op) per out-edge;
    /// stops at the first failing node.
    fn run(&mut self, req: &ChainRequest) -> ChainResult {
        let topo = self.topo;
        let gw = topo.gateway;
        if let Err(status) = self.node(gw, HopKind::Proc) {
            return ChainResult::Err { node: gw, status };
        }
        let Some(svc) = req.entry else {
            return ChainResult::Ok;
        };
        let net = topo.edge(gw, svc).map_or(Dist::ZERO, |e| e.net.clone());
        self.net(svc, &net);
        if let Err(status) = self.node(svc, HopKind::Proc) {
            return ChainResult::Err { node: svc, status };
        }
        for e in topo.out_edges(svc) {
            self.net(e.to, &e.net);
            if let Err(status) = self.node(e.to, HopKind::Op(req.op)) {
                return ChainResult::Err { node: e.to, status };
            }
        }
        ChainResult::Ok
    }

    /// **Private.** Samples an edge's latency and records a `Net` hop.
    fn net(&mut self, to: NodeId, d: &Dist) {
        let dur = d.sample(self.rng);
        self.hops.push(Hop {
            node: to,
            kind: HopKind::Net,
            dur,
        });
    }

    /// **Private.** Chaos check, then the node's own cost. Always draws the error
    /// roll, so the RNG sequence (and every later timing) doesn't depend on chaos
    /// settings.
    fn node(&mut self, id: NodeId, kind: HopKind) -> Result<(), u16> {
        let roll: f64 = self.rng.random();
        let c = self.chaos.nodes.get(&id);
        if let Some(c) = c {
            if c.down {
                return Err(503);
            }
            if roll < c.error_rate {
                return Err(c.error_status);
            }
        }
        let topo = self.topo;
        let n = &topo.nodes[id];
        let dist = match kind {
            HopKind::Op(op) => n.ops.as_ref().map_or(&Dist::ZERO, |o| &o[op as usize]),
            _ => &n.proc,
        };
        let dur = dist.sample(self.rng) + c.map_or(Duration::ZERO, |c| c.latency_add);
        self.hops.push(Hop {
            node: id,
            kind,
            dur,
        });
        Ok(())
    }
}
