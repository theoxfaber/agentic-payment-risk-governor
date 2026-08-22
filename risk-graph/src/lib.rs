//! Risk Evidence Graph — the intelligence plane's data structure.
//!
//! A typed in-memory property graph over payment-domain entities. Not a
//! database: the source of truth is Postgres (later) / event streams (now);
//! this is the queryable projection the investigation engine reasons over.
//!
//! Design rules:
//! - Entities are strongly typed; relationships are strongly typed.
//! - Attributes are open (serde_json values) so ingest never blocks on schema.
//! - Clustering (abuse rings) = union-find over customers linked through
//!   shared resources (device / IP / address / instrument).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Entities
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Merchant,
    Customer,
    Agent,
    Device,
    IpAddress,
    Address,
    PaymentInstrument,
    Payment,
    Order,
    Refund,
    Return,
    Dispute,
}

impl EntityKind {
    pub fn prefix(&self) -> &'static str {
        match self {
            EntityKind::Merchant => "mer",
            EntityKind::Customer => "cus",
            EntityKind::Agent => "agt",
            EntityKind::Device => "dev",
            EntityKind::IpAddress => "ip",
            EntityKind::Address => "adr",
            EntityKind::PaymentInstrument => "pin",
            EntityKind::Payment => "pay",
            EntityKind::Order => "ord",
            EntityKind::Refund => "ref",
            EntityKind::Return => "ret",
            EntityKind::Dispute => "dsp",
        }
    }
}

/// Canonical node identity: kind-prefixed external id, e.g. `cus:CUST_42`.
/// Same external id + same kind = same node; cross-kind ids never collide.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EntityId(pub String);

impl EntityId {
    pub fn new(kind: EntityKind, external: impl AsRef<str>) -> Self {
        EntityId(format!("{}:{}", kind.prefix(), external.as_ref()))
    }

    pub fn generate(kind: EntityKind) -> Self {
        Self::new(kind, Uuid::new_v4().to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: EntityId,
    pub kind: EntityKind,
    #[serde(default)]
    pub attrs: serde_json::Map<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Relationships
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    /// customer -> payment
    Made,
    /// payment -> merchant
    BelongsTo,
    /// customer -> device
    UsesDevice,
    /// customer -> payment instrument
    UsesInstrument,
    /// customer -> address (shipping/billing)
    ShipsTo,
    /// customer/session -> ip
    FromIp,
    /// agent -> payment action (refund/payout request)
    Initiated,
    /// payment -> order
    FulfilledOrder,
    /// order -> return
    Returned,
    /// order -> refund
    RefundedBy,
    /// customer -> dispute
    Raised,
}

impl RelationKind {
    /// Resource-sharing relations used for abuse-ring clustering.
    /// Two customers touching the same resource node via any of these
    /// are joined into the same candidate cluster.
    pub fn linking_kinds() -> &'static [RelationKind] {
        &[
            RelationKind::UsesDevice,
            RelationKind::UsesInstrument,
            RelationKind::ShipsTo,
            RelationKind::FromIp,
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub from: EntityId,
    pub relation: RelationKind,
    pub to: EntityId,
    #[serde(default)]
    pub attrs: serde_json::Map<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Graph
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
pub struct PropertyGraph {
    nodes: HashMap<EntityId, Entity>,
    /// adjacency: node id -> outgoing edge indexes; undirected access via neighbors()
    out_edges: HashMap<EntityId, Vec<usize>>,
    in_edges: HashMap<EntityId, Vec<usize>>,
    edges: Vec<Edge>,
}

#[derive(Debug, Error)]
pub enum GraphError {
    #[error("missing endpoint {0}")]
    MissingEndpoint(String),
}

impl PropertyGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or merge. Merge keeps existing attrs unless overwritten.
    pub fn upsert_node(&mut self, entity: Entity) {
        match self.nodes.get_mut(&entity.id) {
            Some(existing) => {
                for (k, v) in entity.attrs {
                    existing.attrs.insert(k, v);
                }
            }
            None => {
                self.out_edges.entry(entity.id.clone()).or_default();
                self.in_edges.entry(entity.id.clone()).or_default();
                self.nodes.insert(entity.id.clone(), entity);
            }
        }
    }

    pub fn add_edge(
        &mut self,
        from: EntityId,
        relation: RelationKind,
        to: EntityId,
    ) -> Result<(), GraphError> {
        if !self.nodes.contains_key(&from) {
            return Err(GraphError::MissingEndpoint(from.0));
        }
        if !self.nodes.contains_key(&to) {
            return Err(GraphError::MissingEndpoint(to.0));
        }
        let idx = self.edges.len();
        self.edges.push(Edge { from, relation, to, attrs: Default::default() });
        // safe: both endpoints were touched above (or pre-existed)
        let f = self.edges[idx].from.clone();
        let t = self.edges[idx].to.clone();
        self.out_edges.entry(f).or_default().push(idx);
        self.in_edges.entry(t).or_default().push(idx);
        Ok(())
    }

    pub fn node(&self, id: &EntityId) -> Option<&Entity> {
        self.nodes.get(id)
    }

    pub fn nodes(&self) -> impl Iterator<Item = &Entity> {
        self.nodes.values()
    }

    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// All entities connected to `id` in either direction, with the relation seen.
    pub fn neighbors(&self, id: &EntityId) -> Vec<(&Entity, RelationKind, Direction)> {
        let mut out = Vec::new();
        if let Some(idxs) = self.out_edges.get(id) {
            for &i in idxs {
                let e = &self.edges[i];
                if let Some(n) = self.nodes.get(&e.to) {
                    out.push((n, e.relation, Direction::Outgoing));
                }
            }
        }
        if let Some(idxs) = self.in_edges.get(id) {
            for &i in idxs {
                let e = &self.edges[i];
                if let Some(n) = self.nodes.get(&e.from) {
                    out.push((n, e.relation, Direction::Incoming));
                }
            }
        }
        out
    }

    /// Entities of kind K reachable in one hop from `id`.
    pub fn related_of_kind(&self, id: &EntityId, kind: EntityKind) -> Vec<&Entity> {
        self.neighbors(id)
            .into_iter()
            .filter(|(n, _, _)| n.kind == kind)
            .map(|(n, _, _)| n)
            .collect()
    }

    /// All customers sharing at least one linking resource with `customer_id`
    /// (one hop through device/instrument/address/ip).
    pub fn co_customers_via_resources(&self, customer_id: &EntityId) -> Vec<EntityId> {
        let mut found = std::collections::BTreeSet::new();
        for rel in RelationKind::linking_kinds() {
            for idx in self.out_edges.get(customer_id).into_iter().flatten() {
                let e = &self.edges[*idx];
                if e.relation != *rel {
                    continue;
                }
                // resource node -> other incoming UsesX edges -> other customers
                for ridx in self.in_edges.get(&e.to).into_iter().flatten() {
                    let re = &self.edges[*ridx];
                    if re.relation == *rel && re.from != *customer_id {
                        found.insert(re.from.clone());
                    }
                }
            }
        }
        found.into_iter().collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Outgoing,
    Incoming,
}

// ---------------------------------------------------------------------------
// Abuse-ring clustering: union-find over customers joined by shared resources
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Cluster {
    /// Customer entity ids, sorted for deterministic output.
    pub members: Vec<EntityId>,
    /// Shared resource nodes that caused joins within this cluster.
    pub shared_resources: Vec<EntityId>,
    /// Which relation kinds produced links (device vs address vs ...).
    pub link_kinds: Vec<RelationKind>,
}

struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self { parent: (0..n).collect() }
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }

    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent[rb] = ra;
        }
    }
}

impl PropertyGraph {
    /// Cluster all customers into candidate abuse rings by transitive
    /// resource sharing. Deterministic: sorted member lists, stable order.
    ///
    /// NOTE: structural only — a family sharing one laptop lands in the same
    /// cluster as a fraud ring. Separating those is the investigation layer's
    /// job (counter-evidence), not the graph's.
    pub fn abuse_ring_clusters(&self, min_size: usize) -> Vec<Cluster> {
        let customers: Vec<&Entity> = {
            let mut v: Vec<_> = self.nodes.values().filter(|n| n.kind == EntityKind::Customer).collect();
            v.sort_by(|a, b| a.id.0.cmp(&b.id.0));
            v
        };
        let index: HashMap<&EntityId, usize> =
            customers.iter().enumerate().map(|(i, c)| (&c.id, i)).collect();

        let mut uf = UnionFind::new(customers.len());
        let mut shared_resources: HashMap<usize, Vec<EntityId>> = HashMap::new();
        let mut link_kinds_seen: HashMap<usize, std::collections::BTreeSet<RelationKind>> =
            HashMap::new();

        // For every resource node, join all customers pointing at it.
        for rel in RelationKind::linking_kinds() {
            for (ri, edge) in self.edges.iter().enumerate() {
                if edge.relation != *rel {
                    continue;
                }
                // incoming users of this resource
                let users: Vec<usize> = self.in_edges.get(&edge.to).into_iter().flatten()
                    .filter_map(|&i| {
                        let re = &self.edges[i];
                        if re.relation == *rel {
                            index.get(&re.from).copied()
                        } else {
                            None
                        }
                    })
                    .collect();
                if users.len() < 2 || !users.iter().any(|u| customers[*u].kind == EntityKind::Customer) {
                    continue;
                }
                // Only cluster CUSTOMER users
                let cus_users: Vec<usize> = users.into_iter().filter(|u| customers[*u].kind == EntityKind::Customer).collect();
                if cus_users.len() < 2 {
                    continue;
                }
                for &u in &cus_users[1..] {
                    uf.union(cus_users[0], u);
                }
                let root = uf.find(cus_users[0]);
                shared_resources.entry(root).or_default().push(EntityId(self.edges[ri].to.0.clone()));
                link_kinds_seen.entry(root).or_default().insert(*rel);
            }
        }

        // Group by root
        let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
        for i in 0..customers.len() {
            groups.entry(uf.find(i)).or_default().push(i);
        }

        let mut clusters: Vec<Cluster> = groups
            .into_values()
            .filter(|g| g.len() >= min_size)
            .map(|mut g| {
                g.sort(); // by insertion index == sorted by id (we pre-sorted)
                let root = uf.find(g[0]);
                Cluster {
                    members: g.iter().map(|i| customers[*i].id.clone()).collect(),
                    shared_resources: {
                        let mut sr = shared_resources.remove(&root).unwrap_or_default();
                        sr.sort_by(|a, b| a.0.cmp(&b.0));
                        sr.dedup();
                        sr
                    },
                    link_kinds: link_kinds_seen
                        .remove(&root)
                        .map(|s| s.into_iter().collect())
                        .unwrap_or_default(),
                }
            })
            .collect();

        clusters.sort_by(|a, b| b.members.len().cmp(&a.members.len()).then(a.members[0].0.cmp(&b.members[0].0)));
        clusters
    }
}

// ---------------------------------------------------------------------------
// Builder — ergonomic ingest
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
pub struct GraphBuilder {
    graph: PropertyGraph,
}

impl GraphBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn entity(mut self, kind: EntityKind, external_id: impl AsRef<str>) -> Self {
        self.graph.upsert_node(Entity {
            id: EntityId::new(kind, external_id),
            kind,
            attrs: Default::default(),
        });
        self
    }

    pub fn entity_with(
        mut self,
        kind: EntityKind,
        external_id: impl AsRef<str>,
        attrs: serde_json::Value,
    ) -> Self {
        let map = match attrs {
            serde_json::Value::Object(m) => m,
            _ => serde_json::Map::new(),
        };
        self.graph.upsert_node(Entity { id: EntityId::new(kind, external_id), kind, attrs: map });
        self
    }

    pub fn relate(
        mut self,
        from_kind: EntityKind,
        from_ext: impl AsRef<str>,
        rel: RelationKind,
        to_kind: EntityKind,
        to_ext: impl AsRef<str>,
    ) -> Self {
        let from = EntityId::new(from_kind, from_ext);
        let to = EntityId::new(to_kind, to_ext);
        // auto-create missing endpoints as bare nodes so ingest never fails
        for (id, kind) in [(&from, from_kind), (&to, to_kind)] {
            if self.graph.node(id).is_none() {
                self.graph.upsert_node(Entity { id: id.clone(), kind, attrs: Default::default() });
            }
        }
        let _ = self.graph.add_edge(from, rel, to);
        self
    }

    pub fn build(self) -> PropertyGraph {
        self.graph
    }
}
