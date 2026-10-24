//! Where in the config a step sits (for positioned errors), the step key counter and the compile jobs.

use super::{
    EndpointConfig, ErrorMapping, IndexMap, Located, NodeId, Op, PathBuf, PathSeg, Responder, Step,
    StepKey, Topology, VecDeque,
};

/// **Private.** Where a step or entry sits: message prefix, path, file.
#[derive(Clone)]
pub(super) struct Place {
    pub(super) ctx: String,
    pub(super) path: Vec<PathSeg>,
    pub(super) file: Option<PathBuf>,
}

impl Place {
    pub(super) fn key(&self, k: &str) -> Place {
        let mut p = self.clone();
        p.path.push(PathSeg::Key(k.to_string()));
        p
    }

    pub(super) fn index(&self, i: usize) -> Place {
        let mut p = self.clone();
        p.path.push(PathSeg::Index(i));
        p
    }

    /// **Private.** `ctx: msg` at the key or list item.
    pub(super) fn err(&self, msg: impl std::fmt::Display) -> Located {
        self.place(Located::new(
            self.path.clone(),
            format!("{}: {msg}", self.ctx),
        ))
    }

    /// **Private.** `ctx: msg` at the value.
    pub(super) fn bad(&self, msg: impl std::fmt::Display) -> Located {
        self.place(Located::at_value(
            self.path.clone(),
            format!("{}: {msg}", self.ctx),
        ))
    }

    pub(super) fn place(&self, l: Located) -> Located {
        match &self.file {
            Some(f) => l.in_file(f.clone()),
            None => l,
        }
    }
}

/// **Private.** Occurrence counter per target within one sequence (D8).
pub(super) struct Keys(Vec<u32>);

impl Keys {
    pub(super) fn new(nodes: usize) -> Keys {
        Keys(vec![0; nodes])
    }

    /// **Private.** A counter that continues after `steps` (the gateway part of a chain).
    pub(super) fn after(steps: &[Step], nodes: usize) -> Keys {
        let mut k = Keys::new(nodes);
        for s in steps {
            match s {
                Step::Call(c) => {
                    k.next(c.node);
                }
                Step::Cache(c) => {
                    k.next(c.node);
                }
                _ => {}
            }
        }
        k
    }

    pub(super) fn next(&mut self, target: NodeId) -> StepKey {
        let occ = self.0[target];
        self.0[target] += 1;
        StepKey { target, occ }
    }
}

/// **Private.** Nodes callees-first (reverse topological order, ties by id). Iterative
/// (R6.12): Kahn's algorithm on out-degree.
pub(super) fn callees_first(topo: &Topology) -> Vec<NodeId> {
    let n = topo.nodes.len();
    let (mut left, mut callers) = (vec![0usize; n], vec![Vec::new(); n]);
    for e in &topo.edges {
        left[e.from] += 1;
        callers[e.to].push(e.from);
    }
    let mut ready: VecDeque<NodeId> = (0..n).filter(|&i| left[i] == 0).collect();
    let mut order = Vec::with_capacity(n);
    while let Some(x) = ready.pop_front() {
        order.push(x);
        for &c in &callers[x] {
            left[c] -= 1;
            if left[c] == 0 {
                ready.push_back(c);
            }
        }
    }
    order
}

/// **Private.** One endpoint to compile.
pub(super) struct Job<'e> {
    pub(super) key: String,
    pub(super) owner: Option<NodeId>,
    pub(super) op: Op,
    pub(super) cfg: Option<&'e EndpointConfig>,
    pub(super) error_map: IndexMap<String, ErrorMapping>,
    pub(super) place: Place,
    pub(super) responder: Option<Responder>,
}

/// **Private.** A valid `error_map` key, normalized: a node name, a failure kind (`reset`,
/// `malformed`, `timeout`), `400..=599`, `4xx`/`5xx`.
pub(super) fn error_key(k: &str, topo: &Topology) -> Option<String> {
    if topo.id(k).is_some() {
        return Some(k.to_string());
    }
    let lower = k.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "4xx" | "5xx" | "reset" | "malformed" | "timeout"
    ) {
        return Some(lower);
    }
    // Three plain digits: `u16::parse` would also take `+503` and `0503`.
    (k.len() == 3 && k.bytes().all(|b| b.is_ascii_digit()))
        .then(|| k.parse::<u16>().ok())
        .flatten()
        .filter(|s| (400..=599).contains(s))
        .map(|s| s.to_string())
}

/// **Private.** An `endpoints:` entry whose key and values were checked.
pub(super) struct Entry<'e> {
    pub(super) written: String,
    pub(super) method: String,
    pub(super) path: String,
    pub(super) cfg: &'e EndpointConfig,
    pub(super) error_map: IndexMap<String, ErrorMapping>,
    pub(super) place: Place,
}
