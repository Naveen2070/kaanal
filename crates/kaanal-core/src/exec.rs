use std::time::Duration;

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::chaos::ChaosState;
use crate::dist::Dist;
use crate::stable_hash;
use crate::topology::{NodeId, Topology};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Discriminants index `Node::ops` ([read, write, scan]); keep in sync with the topology.
pub enum Op {
    Read = 0,
    Write = 1,
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

    pub fn name(self) -> &'static str {
        match self {
            Op::Read => "read",
            Op::Write => "write",
            Op::Scan => "scan",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HopKind {
    Net,
    Proc,
    Op(Op),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hop {
    pub node: NodeId,
    pub kind: HopKind,
    pub dur: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChainResult {
    Ok,
    Err { node: NodeId, status: u16 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChainOutcome {
    pub hops: Vec<Hop>,
    pub result: ChainResult,
}

impl ChainOutcome {
    pub fn total(&self) -> Duration {
        self.hops.iter().map(|h| h.dur).sum()
    }
}

pub struct ChainRequest<'a> {
    /// Service that owns the route; `None` = static route answered by the gateway itself.
    pub entry: Option<NodeId>,
    pub op: Op,
    pub endpoint_key: &'a str,
    pub n: u64,
    pub seed: u64,
}

/// Pure: computes what the request would experience. The caller sleeps (live) or advances virtual time (sim).
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

struct Walk<'a> {
    topo: &'a Topology,
    chaos: &'a ChaosState,
    rng: &'a mut ChaCha8Rng,
    hops: Vec<Hop>,
}

impl Walk<'_> {
    fn run(&mut self, req: &ChainRequest) -> ChainResult {
        let topo = self.topo;
        let gw = topo.gateway;
        if let Err(status) = self.node(gw, HopKind::Proc) {
            return ChainResult::Err { node: gw, status };
        }
        let Some(svc) = req.entry else {
            return ChainResult::Ok;
        };
        let net = topo
            .edge(gw, svc)
            .map(|e| e.net.clone())
            .unwrap_or(Dist::ZERO);
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

    fn net(&mut self, to: NodeId, d: &Dist) {
        let dur = d.sample(self.rng);
        self.hops.push(Hop {
            node: to,
            kind: HopKind::Net,
            dur,
        });
    }

    /// Chaos check, then the node's own cost. Always draws the error roll,
    /// so the RNG sequence (and every later timing) doesn't depend on chaos settings.
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
            HopKind::Op(op) => n
                .ops
                .as_ref()
                .map(|o| &o[op as usize])
                .unwrap_or(&Dist::ZERO),
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
