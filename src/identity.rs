use blake3::Hasher;

pub type Digest = [u8; 32];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rung<'a> {
    Nominal,
    Api,
    Abi { target: &'a str },
    Object { target: &'a str },
}

impl Rung<'_> {
    fn key(&self) -> &'static str {
        match self {
            Rung::Nominal => "jinn:identity:v1:nominal",
            Rung::Api => "jinn:identity:v1:api",
            Rung::Abi { .. } => "jinn:identity:v1:abi",
            Rung::Object { .. } => "jinn:identity:v1:object",
        }
    }

    fn target(&self) -> Option<&str> {
        match self {
            Rung::Abi { target } | Rung::Object { target } => Some(target),
            Rung::Nominal | Rung::Api => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Node(usize),
    External(Digest),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityError {
    DanglingEdge { from: usize, to: usize },
    EmptyTarget,
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdentityError::DanglingEdge { from, to } => {
                write!(f, "identity graph edge {from} -> {to} names no node")
            }
            IdentityError::EmptyTarget => {
                write!(f, "target-dependent identity rung without a target")
            }
        }
    }
}

#[derive(Default, Clone, Debug)]
pub struct IdentityGraph {
    labels: Vec<Vec<u8>>,
    edges: Vec<Vec<Edge>>,
}

impl IdentityGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn node(&mut self, label: impl Into<Vec<u8>>) -> usize {
        self.labels.push(label.into());
        self.edges.push(Vec::new());
        self.labels.len() - 1
    }

    pub fn edge(&mut self, from: usize, to: Edge) {
        self.edges[from].push(to);
    }

    pub fn len(&self) -> usize {
        self.labels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.labels.is_empty()
    }

    pub fn hashes(&self, rung: Rung<'_>) -> Result<Vec<Digest>, IdentityError> {
        if rung.target().is_some_and(str::is_empty) {
            return Err(IdentityError::EmptyTarget);
        }
        for (from, out) in self.edges.iter().enumerate() {
            for e in out {
                if let Edge::Node(to) = *e
                    && to >= self.len()
                {
                    return Err(IdentityError::DanglingEdge { from, to });
                }
            }
        }
        let comp = self.components();
        let mut pos = vec![0; self.len()];
        for scc in &comp.order {
            for (i, &n) in scc.iter().enumerate() {
                pos[n] = i;
            }
        }
        let mut done: Vec<Option<Digest>> = vec![None; self.len()];
        for scc in &comp.order {
            let hs: Vec<Digest> = scc
                .iter()
                .map(|&r| self.rooted_hash(rung, r, scc.len(), &comp.of, &pos, &done))
                .collect();
            for (&n, h) in scc.iter().zip(hs) {
                done[n] = Some(h);
            }
        }
        Ok(done
            .into_iter()
            .map(|h| h.expect("every node lies in one SCC"))
            .collect())
    }

    fn rooted_hash(
        &self,
        rung: Rung<'_>,
        root: usize,
        size: usize,
        of: &[usize],
        pos: &[usize],
        done: &[Option<Digest>],
    ) -> Digest {
        let scc = of[root];
        let mut local: Vec<Option<usize>> = vec![None; size];
        let mut visit = vec![root];
        local[pos[root]] = Some(0);
        let mut i = 0;
        while i < visit.len() {
            let n = visit[i];
            for e in &self.edges[n] {
                if let Edge::Node(t) = *e
                    && of[t] == scc
                    && local[pos[t]].is_none()
                {
                    local[pos[t]] = Some(visit.len());
                    visit.push(t);
                }
            }
            i += 1;
        }
        let mut h = Hasher::new_derive_key(rung.key());
        if let Some(t) = rung.target() {
            put_bytes(&mut h, t.as_bytes());
        }
        put_len(&mut h, visit.len());
        for &n in &visit {
            put_bytes(&mut h, &self.labels[n]);
            put_len(&mut h, self.edges[n].len());
            for e in &self.edges[n] {
                match *e {
                    Edge::Node(t) if of[t] == scc => {
                        h.update(&[0]);
                        put_len(
                            &mut h,
                            local[pos[t]].expect("SCC member reached from its root"),
                        );
                    }
                    Edge::Node(t) => {
                        h.update(&[1]);
                        h.update(&done[t].expect("successor SCC hashed first"));
                    }
                    Edge::External(d) => {
                        h.update(&[1]);
                        h.update(&d);
                    }
                }
            }
        }
        *h.finalize().as_bytes()
    }

    fn components(&self) -> Components {
        let n = self.len();
        let mut index = vec![usize::MAX; n];
        let mut low = vec![0; n];
        let mut on_stack = vec![false; n];
        let mut stack = Vec::new();
        let mut of = vec![usize::MAX; n];
        let mut order = Vec::new();
        let mut next = 0;
        for start in 0..n {
            if index[start] != usize::MAX {
                continue;
            }
            let mut frames: Vec<(usize, usize)> = vec![(start, 0)];
            index[start] = next;
            low[start] = next;
            next += 1;
            stack.push(start);
            on_stack[start] = true;
            while let Some(&mut (v, ref mut ei)) = frames.last_mut() {
                if let Some(e) = self.edges[v].get(*ei) {
                    *ei += 1;
                    if let Edge::Node(w) = *e {
                        if index[w] == usize::MAX {
                            index[w] = next;
                            low[w] = next;
                            next += 1;
                            stack.push(w);
                            on_stack[w] = true;
                            frames.push((w, 0));
                        } else if on_stack[w] {
                            low[v] = low[v].min(index[w]);
                        }
                    }
                    continue;
                }
                frames.pop();
                if let Some(&(p, _)) = frames.last() {
                    low[p] = low[p].min(low[v]);
                }
                if low[v] == index[v] {
                    let id = order.len();
                    let mut scc = Vec::new();
                    loop {
                        let w = stack.pop().expect("Tarjan stack holds the component");
                        on_stack[w] = false;
                        of[w] = id;
                        scc.push(w);
                        if w == v {
                            break;
                        }
                    }
                    order.push(scc);
                }
            }
        }
        Components { of, order }
    }
}

struct Components {
    of: Vec<usize>,
    order: Vec<Vec<usize>>,
}

fn put_len(h: &mut Hasher, n: usize) {
    h.update(&(n as u64).to_le_bytes());
}

fn put_bytes(h: &mut Hasher, b: &[u8]) {
    put_len(h, b.len());
    h.update(b);
}

#[cfg(test)]
mod tests {
    use super::*;

    const API: Rung<'static> = Rung::Api;

    fn build(
        labels: &[&str],
        edges: &[(usize, usize)],
        perm: &[usize],
    ) -> (IdentityGraph, Vec<usize>) {
        let mut g = IdentityGraph::new();
        let mut at = vec![0; labels.len()];
        for &p in perm {
            at[p] = g.node(labels[p]);
        }
        for &p in perm {
            for &(f, t) in edges {
                if f == p {
                    g.edge(at[f], Edge::Node(at[t]));
                }
            }
        }
        (g, at)
    }

    fn logical(labels: &[&str], edges: &[(usize, usize)], perm: &[usize]) -> Vec<Digest> {
        let (g, at) = build(labels, edges, perm);
        let hs = g.hashes(API).unwrap();
        at.iter().map(|&i| hs[i]).collect()
    }

    #[test]
    fn insertion_order_does_not_change_hashes() {
        let labels = ["List", "Node", "Leaf", "i64"];
        let edges = [(0, 1), (1, 3), (1, 0), (0, 2), (2, 3)];
        let base = logical(&labels, &edges, &[0, 1, 2, 3]);
        for perm in [[3, 2, 1, 0], [1, 3, 0, 2], [2, 0, 3, 1]] {
            assert_eq!(logical(&labels, &edges, &perm), base);
        }
    }

    #[test]
    fn edge_order_is_identity_bearing() {
        let a = logical(&["P", "x", "y"], &[(0, 1), (0, 2)], &[0, 1, 2]);
        let b = logical(&["P", "x", "y"], &[(0, 2), (0, 1)], &[0, 1, 2]);
        assert_ne!(a[0], b[0]);
    }

    #[test]
    fn symmetric_cycle_members_share_structure_and_differ_by_name() {
        let sym = logical(&["T", "T"], &[(0, 1), (1, 0)], &[0, 1]);
        assert_eq!(sym[0], sym[1]);
        let named = logical(&["A", "B"], &[(0, 1), (1, 0)], &[0, 1]);
        assert_ne!(named[0], named[1]);
    }

    #[test]
    fn cycle_length_is_distinguished() {
        let one = logical(&["T"], &[(0, 0)], &[0]);
        let two = logical(&["T", "T"], &[(0, 1), (1, 0)], &[0, 1]);
        assert_ne!(one[0], two[0]);
    }

    #[test]
    fn dependency_change_propagates_through_cycle() {
        let a = logical(&["A", "B", "i32"], &[(0, 1), (1, 0), (1, 2)], &[0, 1, 2]);
        let b = logical(&["A", "B", "i64"], &[(0, 1), (1, 0), (1, 2)], &[0, 1, 2]);
        assert_ne!(a[0], b[0]);
        assert_ne!(a[1], b[1]);
    }

    #[test]
    fn external_hash_participates() {
        let mut g = IdentityGraph::new();
        let n = g.node("F");
        g.edge(n, Edge::External([1; 32]));
        let mut h = IdentityGraph::new();
        let m = h.node("F");
        h.edge(m, Edge::External([2; 32]));
        assert_ne!(g.hashes(API).unwrap()[n], h.hashes(API).unwrap()[m]);
    }

    #[test]
    fn edge_to_hashed_node_equals_external_digest() {
        let mut g = IdentityGraph::new();
        let leaf = g.node("i64");
        let top = g.node("Box");
        g.edge(top, Edge::Node(leaf));
        let hs = g.hashes(API).unwrap();
        let mut h = IdentityGraph::new();
        let t = h.node("Box");
        h.edge(t, Edge::External(hs[leaf]));
        assert_eq!(h.hashes(API).unwrap()[t], hs[top]);
    }

    #[test]
    fn label_boundaries_are_unambiguous() {
        let a = logical(&["ab", "c"], &[(0, 1)], &[0, 1]);
        let b = logical(&["a", "bc"], &[(0, 1)], &[0, 1]);
        assert_ne!(a[0], b[0]);
    }

    #[test]
    fn rungs_and_targets_are_domain_separated() {
        let mut g = IdentityGraph::new();
        let n = g.node("T");
        let at = |r| g.hashes(r).unwrap()[n];
        let x86 = Rung::Abi {
            target: "x86_64-unknown-linux-gnu",
        };
        let arm = Rung::Abi {
            target: "aarch64-unknown-linux-gnu",
        };
        let obj = Rung::Object {
            target: "x86_64-unknown-linux-gnu",
        };
        let all = [at(Rung::Nominal), at(Rung::Api), at(x86), at(arm), at(obj)];
        for i in 0..all.len() {
            for j in i + 1..all.len() {
                assert_ne!(all[i], all[j]);
            }
        }
        assert_eq!(
            g.hashes(Rung::Abi { target: "" }),
            Err(IdentityError::EmptyTarget)
        );
    }

    #[test]
    fn dangling_edge_is_rejected() {
        let mut g = IdentityGraph::new();
        let n = g.node("T");
        g.edge(n, Edge::Node(7));
        assert_eq!(
            g.hashes(API),
            Err(IdentityError::DanglingEdge { from: 0, to: 7 })
        );
    }

    #[test]
    fn deep_chain_does_not_recurse() {
        let mut g = IdentityGraph::new();
        let mut prev = g.node("end");
        for _ in 0..200_000 {
            let n = g.node("link");
            g.edge(n, Edge::Node(prev));
            prev = n;
        }
        let hs = g.hashes(API).unwrap();
        assert_eq!(hs.len(), 200_001);
        let mut ring = IdentityGraph::new();
        let first = ring.node("r");
        let mut last = first;
        for _ in 1..2_000 {
            let n = ring.node("r");
            ring.edge(last, Edge::Node(n));
            last = n;
        }
        ring.edge(last, Edge::Node(first));
        let rs = ring.hashes(API).unwrap();
        assert!(rs.iter().all(|h| *h == rs[0]));
    }

    #[test]
    fn pinned_vector_is_stable_across_processes() {
        let hs = logical(&["A", "B"], &[(0, 1), (1, 0)], &[0, 1]);
        let mut h = Hasher::new_derive_key("jinn:identity:v1:api");
        put_len(&mut h, 2);
        put_bytes(&mut h, b"A");
        put_len(&mut h, 1);
        h.update(&[0]);
        put_len(&mut h, 1);
        put_bytes(&mut h, b"B");
        put_len(&mut h, 1);
        h.update(&[0]);
        put_len(&mut h, 0);
        assert_eq!(hs[0], *h.finalize().as_bytes());
    }
}
