use kaanal_core::{Config, NodeKind, Topology, TopologyError};

const BASE: &str = r"
version: 1
nodes:
  nginx: { type: gateway }
  app:   { type: service, spec: openapi.yaml }
  db:    { type: database, ops: { read: 4ms, write: 8ms, scan: 30ms } }
edges:
  - { from: nginx, to: app }
  - { from: app, to: db }
";

fn build(yaml: &str) -> Result<Topology, TopologyError> {
    Topology::build(&Config::from_yaml(yaml).unwrap())
}

#[test]
fn builds_base() {
    let t = build(BASE).unwrap();
    assert_eq!(t.nodes.len(), 3);
    assert_eq!(t.nodes[t.gateway].name, "nginx");
    let app = t.id("app").unwrap();
    let db = t.id("db").unwrap();
    assert_eq!(t.nodes[db].kind, NodeKind::Database);
    assert!(t.edge(t.gateway, app).is_some());
    assert_eq!(t.out_edges(app).map(|e| e.to).collect::<Vec<_>>(), vec![db]);
}

#[test]
fn rejects_cycle() {
    let yaml = format!("{BASE}  - {{ from: db, to: app }}\n");
    assert_eq!(
        build(&yaml).unwrap_err(),
        TopologyError::Cycle(vec!["app".into(), "db".into(), "app".into()])
    );
}

#[test]
fn rejects_unknown_node_in_edge() {
    let yaml = format!("{BASE}  - {{ from: app, to: redis }}\n");
    assert_eq!(
        build(&yaml).unwrap_err(),
        TopologyError::UnknownNode {
            edge: 2,
            name: "redis".into()
        }
    );
}

#[test]
fn rejects_two_gateways() {
    let yaml = BASE.replace("nodes:\n", "nodes:\n  lb: { type: gateway }\n");
    assert_eq!(build(&yaml).unwrap_err(), TopologyError::GatewayCount(2));
}

#[test]
fn rejects_zero_gateways() {
    let yaml = BASE.replace("nginx: { type: gateway }", "nginx: { type: service }");
    assert_eq!(build(&yaml).unwrap_err(), TopologyError::GatewayCount(0));
}

#[test]
fn rejects_database_without_ops() {
    let yaml = BASE.replace(
        "db:    { type: database, ops: { read: 4ms, write: 8ms, scan: 30ms } }",
        "db: { type: database }",
    );
    assert_eq!(
        build(&yaml).unwrap_err(),
        TopologyError::MissingOps("db".into())
    );
}

#[test]
fn rejects_service_spec_without_gateway_edge() {
    let yaml = BASE.replace("  - { from: nginx, to: app }\n", "");
    assert_eq!(
        build(&yaml).unwrap_err(),
        TopologyError::MissingGatewayEdge("app".into())
    );
}

#[test]
fn rejects_service_calling_service_in_phase_one() {
    let yaml = BASE.replace("nodes:\n", "nodes:\n  auth: { type: service }\n")
        + "  - { from: app, to: auth }\n";
    assert!(matches!(
        build(&yaml).unwrap_err(),
        TopologyError::Unsupported(_)
    ));
}

#[test]
fn errors_display_human_text() {
    let e = TopologyError::Cycle(vec!["a".into(), "b".into(), "a".into()]);
    assert_eq!(e.to_string(), "cycle: a -> b -> a");
}

#[test]
fn rejects_duplicate_edge() {
    let yaml = format!(
        "{BASE}  - {{ from: app, to: db }}
"
    );
    assert!(
        matches!(build(&yaml).unwrap_err(), TopologyError::Unsupported(m) if m.contains("duplicate"))
    );
}

#[test]
fn rejects_edges_the_executor_would_ignore() {
    let yaml = format!(
        "{BASE}  - {{ from: nginx, to: db }}
"
    );
    assert!(
        matches!(build(&yaml).unwrap_err(), TopologyError::Unsupported(m) if m.contains("nginx"))
    );
}

#[test]
fn rejects_ops_on_non_database_and_spec_on_non_service() {
    let yaml = BASE.replace(
        "app:   { type: service,",
        "app:   { type: service, ops: { read: 1ms, write: 1ms, scan: 1ms },",
    );
    assert!(
        matches!(build(&yaml).unwrap_err(), TopologyError::Unsupported(m) if m.contains("ops"))
    );
    let yaml = BASE.replace(
        "nginx: { type: gateway }",
        "nginx: { type: gateway, spec: x.yaml }",
    );
    assert!(
        matches!(build(&yaml).unwrap_err(), TopologyError::Unsupported(m) if m.contains("spec"))
    );
}
