//! The road network as a graph, and the evacuation routing on top of it.
//!
//! Agents do not move across open ground toward a bearing; they move along the
//! real OSM network, which is the whole reason the drivable/track split was
//! kept in [`scenario::vectors`]. An engine and a family car are confined to
//! drivable ways; a person on foot can also use tracks, paths and steps.
//!
//! Routing is deliberately **not** per-agent pathfinding. Every agent wants
//! the same thing -- the nearest reachable refuge -- so one multi-source
//! Dijkstra from all refuges gives every node in the network its distance to
//! safety and its next hop toward it. That is a single graph search per
//! refresh instead of 750, and it makes rerouting around a road the fire has
//! cut a property of the field rather than something each agent has to
//! discover. Two fields are kept, because the two travel modes see different
//! graphs.

use std::collections::BinaryHeap;

use scenario::{Pos, Scenario};

use crate::traffic::RoadClass;

/// Snap tolerance when welding way vertices into shared nodes, in metres.
/// OSM ways that meet at a junction share a node exactly, so this only has to
/// absorb the two-decimal rounding in the baked file.
const SNAP_M: f32 = 0.05;

/// Bucket edge for the nearest-node lookup, metres.
const BUCKET_M: f32 = 100.0;

pub type NodeId = u32;
pub const NO_NODE: NodeId = u32::MAX;

#[derive(Debug, Clone, Copy)]
pub struct Edge {
    pub to: NodeId,
    pub length_m: f32,
    /// A vehicle can use this.
    pub drivable: bool,
    /// Index into the network's edge list, for congestion accounting.
    pub id: u32,
}

pub struct RoadNetwork {
    pub nodes: Vec<Pos>,
    /// Ground elevation at each node, so agents follow the terrain.
    pub elev: Vec<f32>,
    adjacency: Vec<Vec<Edge>>,
    pub edge_count: usize,
    /// The two nodes of each undirected edge, low id first, so a directed link
    /// can be turned back into a line on the ground.
    edge_nodes: Vec<[NodeId; 2]>,
    edge_len: Vec<f32>,
    /// [`RoadClass`] of each edge, as `u8`. Kept because the traffic model
    /// needs a capacity and a free-flow speed per road, and the OSM tag that
    /// says so was previously discarded here — every drivable way, from the A10
    /// to a farm service track, became the same edge.
    edge_class: Vec<u8>,
    buckets: Vec<Vec<NodeId>>,
    bcols: usize,
    brows: usize,
    /// Connected component of each node, per travel mode.
    ///
    /// Computed once at build because "is there any road from here to there"
    /// has to be an O(1) question. It is asked by every unit that gets an order,
    /// and the answer is usually *no* for the obvious candidate: the nearest
    /// drivable node to a point up in the macchia is routinely an isolated farm
    /// track that touches nothing. Discovering that with a failed A* — which
    /// explores the whole component before giving up — is how the first version
    /// of this managed to leave every engine parked at staging.
    comp_drive: Vec<u32>,
    comp_walk: Vec<u32>,
}

impl RoadNetwork {
    /// Build from the scenario's OSM ways. Footways, steps and tracks are
    /// included: they are how people leave when the road is blocked.
    pub fn build(scn: &Scenario) -> RoadNetwork {
        let w = scn.world;
        let bcols = (w.width_m / BUCKET_M).ceil() as usize + 1;
        let brows = (w.height_m / BUCKET_M).ceil() as usize + 1;

        let mut net = RoadNetwork {
            nodes: Vec::new(),
            elev: Vec::new(),
            adjacency: Vec::new(),
            edge_count: 0,
            edge_nodes: Vec::new(),
            edge_len: Vec::new(),
            edge_class: Vec::new(),
            buckets: vec![Vec::new(); brows * bcols],
            bcols,
            brows,
            comp_drive: Vec::new(),
            comp_walk: Vec::new(),
        };

        // Weld coincident vertices by exact key. Ways that cross without a
        // shared node are genuinely not connected in OSM either -- that is a
        // bridge or an underpass, and pretending otherwise would route people
        // through a viaduct pier.
        let mut index: std::collections::HashMap<(i64, i64), NodeId> =
            std::collections::HashMap::new();
        let key = |p: [f32; 2]| {
            (
                (p[0] / SNAP_M).round() as i64,
                (p[1] / SNAP_M).round() as i64,
            )
        };

        for road in &scn.vectors.roads {
            if road.line.len() < 2 {
                continue;
            }
            let drivable = road.drivable;
            let class = RoadClass::from_osm(&road.class, drivable);
            let mut prev: Option<NodeId> = None;
            for v in &road.line {
                let k = key(*v);
                let id = *index.entry(k).or_insert_with(|| {
                    let p = Pos { x: v[0], y: v[1] };
                    net.nodes.push(p);
                    // the published ground: the road grading is drawn, not simulated
                    // (`scenario::grade`, until the user decides otherwise)
                    net.elev.push(scn.terrain.raw_height_at(p));
                    net.adjacency.push(Vec::new());
                    (net.nodes.len() - 1) as NodeId
                });
                if let Some(a) = prev {
                    if a != id {
                        net.add_edge(a, id, drivable, class);
                    }
                }
                prev = Some(id);
            }
        }

        for (i, p) in net.nodes.iter().enumerate() {
            let bx = (p.x / BUCKET_M).max(0.0) as usize;
            let by = (p.y / BUCKET_M).max(0.0) as usize;
            if bx < bcols && by < brows {
                net.buckets[by * bcols + bx].push(i as NodeId);
            }
        }
        net.comp_drive = net.components(true);
        net.comp_walk = net.components(false);
        net
    }

    /// Label every node with its connected component, for one travel mode.
    /// Flood fill with an explicit stack: the walkable graph is one 61 k-node
    /// component in places and recursion would blow the stack on it.
    fn components(&self, drivable_only: bool) -> Vec<u32> {
        const NONE: u32 = u32::MAX;
        let mut comp = vec![NONE; self.nodes.len()];
        let mut next = 0u32;
        let mut stack = Vec::new();
        for start in 0..self.nodes.len() as NodeId {
            if comp[start as usize] != NONE {
                continue;
            }
            // A node with no edges of this mode is its own island, which is the
            // correct answer for a footpath vertex asked about driving.
            comp[start as usize] = next;
            stack.push(start);
            while let Some(n) = stack.pop() {
                for e in &self.adjacency[n as usize] {
                    if drivable_only && !e.drivable {
                        continue;
                    }
                    if comp[e.to as usize] == NONE {
                        comp[e.to as usize] = next;
                        stack.push(e.to);
                    }
                }
            }
            next += 1;
        }
        comp
    }

    /// Which connected component a node is in, for a travel mode. Two nodes
    /// with the same label have *some* route between them; whether it is still
    /// open is a question for [`route`].
    pub fn component(&self, n: NodeId, drivable_only: bool) -> u32 {
        let comp = if drivable_only { &self.comp_drive } else { &self.comp_walk };
        comp[n as usize]
    }

    /// Number of connected components, for the diagnostics that make the
    /// disconnected-stub problem visible instead of mysterious.
    pub fn component_count(&self, drivable_only: bool) -> usize {
        let comp = if drivable_only { &self.comp_drive } else { &self.comp_walk };
        comp.iter().copied().max().map(|m| m as usize + 1).unwrap_or(0)
    }

    fn add_edge(&mut self, a: NodeId, b: NodeId, drivable: bool, class: RoadClass) {
        let pa = self.nodes[a as usize];
        let pb = self.nodes[b as usize];
        let len = ((pb.x - pa.x).powi(2) + (pb.y - pa.y).powi(2)).sqrt().max(0.1);
        let id = self.edge_count as u32;
        self.edge_count += 1;
        self.edge_nodes.push([a.min(b), a.max(b)]);
        self.edge_len.push(len);
        self.edge_class.push(class as u8);
        self.adjacency[a as usize].push(Edge { to: b, length_m: len, drivable, id });
        self.adjacency[b as usize].push(Edge { to: a, length_m: len, drivable, id });
    }

    pub fn edge_len(&self, edge: u32) -> f32 {
        self.edge_len[edge as usize]
    }

    pub fn edge_class(&self, edge: u32) -> RoadClass {
        RoadClass::from_u8(self.edge_class[edge as usize])
    }

    /// The two ends of a directed link (`2·edge + dir`), in the order a vehicle
    /// on it travels them.
    pub fn link_ends(&self, link: u32) -> (NodeId, NodeId) {
        let [lo, hi] = self.edge_nodes[(link / 2) as usize];
        if link % 2 == 0 {
            (lo, hi)
        } else {
            (hi, lo)
        }
    }

    /// The undirected edge joining two adjacent nodes, if they are adjacent.
    /// Tolerates [`NO_NODE`] on either side: a unit that has not reached the
    /// network yet has no `at_node`, and asking is more natural at the call
    /// site than checking first.
    pub fn edge_between(&self, a: NodeId, b: NodeId) -> Option<u32> {
        if a == NO_NODE || b == NO_NODE {
            return None;
        }
        self.adjacency[a as usize].iter().find(|e| e.to == b).map(|e| e.id)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn pos(&self, n: NodeId) -> Pos {
        self.nodes[n as usize]
    }

    pub fn neighbours(&self, n: NodeId) -> &[Edge] {
        &self.adjacency[n as usize]
    }

    /// True if any way at this node can carry a vehicle.
    pub fn is_drivable_node(&self, n: NodeId) -> bool {
        self.adjacency[n as usize].iter().any(|e| e.drivable)
    }

    /// Nearest node to `p` that a unit standing at `from` can actually get to.
    ///
    /// The distinction from [`nearest`] is the whole difference between an
    /// engine that responds and one that sits at staging: the closest drivable
    /// node to a point inland is very often a stub of farm track connected to
    /// nothing. This is "drive as close as the road network gets", which is what
    /// an engine crew does, and it is why the caller then has to bridge the
    /// remaining metres with hose rather than with wheels.
    pub fn nearest_reachable(
        &self,
        p: Pos,
        drivable_only: bool,
        from: NodeId,
    ) -> Option<NodeId> {
        if from == NO_NODE {
            return None;
        }
        let want = self.component(from, drivable_only);
        self.nearest_where(p, drivable_only, |n| {
            self.component(n, drivable_only) == want
        })
    }

    /// Nearest node to a position, optionally restricted to drivable ways.
    /// Searches outward by bucket ring so a house up a footpath still finds
    /// the network it is actually on.
    pub fn nearest(&self, p: Pos, drivable_only: bool) -> Option<NodeId> {
        self.nearest_where(p, drivable_only, |_| true)
    }

    fn nearest_where(
        &self,
        p: Pos,
        drivable_only: bool,
        accept: impl Fn(NodeId) -> bool,
    ) -> Option<NodeId> {
        let bx = (p.x / BUCKET_M).max(0.0) as isize;
        let by = (p.y / BUCKET_M).max(0.0) as isize;
        let mut best = None;
        let mut best_d = f32::MAX;

        for ring in 0..=25isize {
            for dy in -ring..=ring {
                for dx in -ring..=ring {
                    // Only the shell of the ring is new.
                    if ring > 0 && dx.abs() != ring && dy.abs() != ring {
                        continue;
                    }
                    let (nx, ny) = (bx + dx, by + dy);
                    if nx < 0 || ny < 0 || nx as usize >= self.bcols || ny as usize >= self.brows
                    {
                        continue;
                    }
                    for &n in &self.buckets[ny as usize * self.bcols + nx as usize] {
                        if drivable_only && !self.is_drivable_node(n) {
                            continue;
                        }
                        if !accept(n) {
                            continue;
                        }
                        let q = self.nodes[n as usize];
                        let d = (q.x - p.x).powi(2) + (q.y - p.y).powi(2);
                        if d < best_d {
                            best_d = d;
                            best = Some(n);
                        }
                    }
                }
            }
            // One extra ring after the first hit, since a nearer node can sit
            // in a diagonal bucket.
            if best.is_some() && (ring as f32 * BUCKET_M).powi(2) > best_d {
                break;
            }
        }
        best
    }
}

/// Where safety is, and how to get there from anywhere on the network.
///
/// `cost` is minutes-equivalent travel time to the nearest refuge, with fire
/// danger folded in as a penalty rather than as a hard block wherever
/// possible, so a route that is merely unpleasant still beats no route at all.
pub struct RouteField {
    /// Travel cost to the nearest refuge, in seconds. `f32::INFINITY` where
    /// no route exists.
    pub cost: Vec<f32>,
    /// Next node on the way to that refuge, or [`NO_NODE`].
    pub next: Vec<NodeId>,
}

impl RouteField {
    fn empty(n: usize) -> RouteField {
        RouteField { cost: vec![f32::INFINITY; n], next: vec![NO_NODE; n] }
    }

    pub fn reachable(&self, n: NodeId) -> bool {
        self.cost[n as usize].is_finite()
    }
}

/// Free-travel speeds, m/s. Foot speed is per-person; these are the network
/// speeds used for *routing* cost, which only needs to rank routes.
const CAR_SPEED: f32 = 11.0;
const FOOT_SPEED: f32 = 1.3;

/// How much a dangerous edge is inflated in routing cost. High enough that a
/// long way round beats driving past the flaming front, low enough that a
/// slightly smoky road is still used.
const DANGER_PENALTY: f32 = 40.0;

/// Danger above which an edge is dropped from routing entirely.
const DANGER_CUT: f32 = fire::threat::IMPASSABLE;

/// Multi-source Dijkstra from every refuge, once per mode.
pub fn solve(
    net: &RoadNetwork,
    refuges: &[NodeId],
    threat: &fire::ThreatField,
    drivable_only: bool,
) -> RouteField {
    solve_with(net, refuges, threat, drivable_only, &[])
}

/// The same, with a set of links closed to traffic by order.
///
/// `closed` is indexed by [`Edge::id`] and may be empty, which is the usual
/// case. A closure is deliberately a property of the *field* rather than of the
/// network: the fire cutting a road and the commander closing one produce the
/// same rerouting through the same machinery, which is the whole reason
/// routing is a field and not a search (finding 8), and a unit or a pedestrian
/// asking [`route`] is unaffected — a barricade stops civilian traffic, not the
/// engine that asked for it and not somebody on foot.
pub fn solve_with(
    net: &RoadNetwork,
    refuges: &[NodeId],
    threat: &fire::ThreatField,
    drivable_only: bool,
    closed: &[bool],
) -> RouteField {
    let n = net.len();
    let mut field = RouteField::empty(n);
    let mut heap: BinaryHeap<Entry> = BinaryHeap::new();

    for &r in refuges {
        if drivable_only && !net.is_drivable_node(r) {
            continue;
        }
        // A refuge the fire has already reached is not a refuge.
        if threat.at(net.pos(r)) >= DANGER_CUT {
            continue;
        }
        field.cost[r as usize] = 0.0;
        heap.push(Entry { cost: 0.0, node: r });
    }

    let speed = if drivable_only { CAR_SPEED } else { FOOT_SPEED };

    while let Some(Entry { cost, node }) = heap.pop() {
        if cost > field.cost[node as usize] {
            continue;
        }
        for e in net.neighbours(node) {
            if drivable_only && !e.drivable {
                continue;
            }
            // A closure is a traffic order, so it only binds the mode it was
            // issued against: closing the Aurelia does not stop anyone walking
            // down it, and modelling it as if it did would make the lever look
            // far worse than it is.
            if drivable_only && closed.get(e.id as usize).copied().unwrap_or(false) {
                continue;
            }
            let mid = midpoint(net.pos(node), net.pos(e.to));
            let danger = threat.at(mid).max(threat.at(net.pos(e.to)));
            if danger >= DANGER_CUT {
                continue;
            }
            let step = e.length_m / speed * (1.0 + DANGER_PENALTY * danger);
            let next = cost + step;
            if next < field.cost[e.to as usize] {
                field.cost[e.to as usize] = next;
                // Edges are undirected, so the predecessor in the search *is*
                // the next hop toward the refuge.
                field.next[e.to as usize] = node;
                heap.push(Entry { cost: next, node: e.to });
            }
        }
    }
    field
}

fn midpoint(a: Pos, b: Pos) -> Pos {
    Pos { x: (a.x + b.x) * 0.5, y: (a.y + b.y) * 0.5 }
}

/// Shortest route from `from` to `to`, as the nodes to visit in order.
///
/// The deliberate opposite of [`solve`], and for a reason worth stating: the
/// civilian model never needs this, because every civilian wants the *same*
/// destination and one multi-source field answers all 750 of them at once. A
/// suppression unit is the other case entirely — a dozen units, each sent to a
/// different point the commander picked — so a per-unit search is both correct
/// and cheap. Twelve A* searches on 61 k nodes cost less than one of the
/// multi-source Dijkstras the civilians already pay for every simulated minute.
///
/// Danger is a cost penalty, not a wall, up to [`DANGER_CUT`]: an engine will
/// drive a smoky road to get to work, and will not drive into the flame front.
pub fn route(
    net: &RoadNetwork,
    from: NodeId,
    to: NodeId,
    threat: &fire::ThreatField,
    drivable_only: bool,
) -> Option<Vec<NodeId>> {
    if from == NO_NODE || to == NO_NODE {
        return None;
    }
    if from == to {
        return Some(Vec::new());
    }
    let n = net.len();
    let speed = if drivable_only { CAR_SPEED } else { FOOT_SPEED };
    let goal = net.pos(to);
    // Straight-line time at free speed: admissible, since no edge can be
    // travelled faster than that and every penalty is >= 1.
    let heuristic = |id: NodeId| {
        let p = net.pos(id);
        ((p.x - goal.x).powi(2) + (p.y - goal.y).powi(2)).sqrt() / speed
    };

    let mut cost = vec![f32::INFINITY; n];
    let mut prev = vec![NO_NODE; n];
    let mut heap: BinaryHeap<Entry> = BinaryHeap::new();
    cost[from as usize] = 0.0;
    heap.push(Entry { cost: heuristic(from), node: from });

    while let Some(Entry { node, .. }) = heap.pop() {
        if node == to {
            break;
        }
        let here = cost[node as usize];
        for e in net.neighbours(node) {
            if drivable_only && !e.drivable {
                continue;
            }
            let mid = midpoint(net.pos(node), net.pos(e.to));
            let danger = threat.at(mid).max(threat.at(net.pos(e.to)));
            // The destination itself may be hot -- that is often exactly where
            // the work is -- so only *intermediate* nodes are refused.
            if danger >= DANGER_CUT && e.to != to {
                continue;
            }
            let step = e.length_m / speed * (1.0 + DANGER_PENALTY * danger);
            let next = here + step;
            if next < cost[e.to as usize] {
                cost[e.to as usize] = next;
                prev[e.to as usize] = node;
                heap.push(Entry { cost: next + heuristic(e.to), node: e.to });
            }
        }
    }

    if !cost[to as usize].is_finite() {
        return None;
    }
    let mut path = vec![to];
    let mut at = to;
    while let Some(p) = Some(prev[at as usize]).filter(|p| *p != NO_NODE) {
        if p == from {
            break;
        }
        path.push(p);
        at = p;
    }
    path.reverse();
    Some(path)
}

/// Every node [`route`] can reach from `from`, by the same rule: an edge whose
/// danger reaches [`DANGER_CUT`] is refused, except as the last one of the way
/// (a hot destination is often exactly where the work is).
pub fn reachable_from(
    net: &RoadNetwork,
    from: NodeId,
    threat: &fire::ThreatField,
    drivable_only: bool,
) -> Vec<bool> {
    let mut seen = vec![false; net.len()];
    if from == NO_NODE {
        return seen;
    }
    // only nodes reached by a safe edge are passed through
    let mut through = vec![false; net.len()];
    seen[from as usize] = true;
    through[from as usize] = true;
    let mut stack = vec![from];
    while let Some(node) = stack.pop() {
        for e in net.neighbours(node) {
            if drivable_only && !e.drivable {
                continue;
            }
            seen[e.to as usize] = true;
            if through[e.to as usize] {
                continue;
            }
            let mid = midpoint(net.pos(node), net.pos(e.to));
            if threat.at(mid).max(threat.at(net.pos(e.to))) < DANGER_CUT {
                through[e.to as usize] = true;
                stack.push(e.to);
            }
        }
    }
    seen
}

/// Min-heap entry. `BinaryHeap` is a max-heap and `f32` is not `Ord`, so the
/// ordering is reversed and NaN-free costs are compared by `partial_cmp`.
struct Entry {
    cost: f32,
    node: NodeId,
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.cost == other.cost
    }
}
impl Eq for Entry {}
impl Ord for Entry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other
            .cost
            .partial_cmp(&self.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}
impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
