use kaanal_data::ApiModel;
use std::fmt::Write as _;

const SPEC: &str = r##"
openapi: 3.0.3
info: { title: t, version: "1" }
paths:
  /users:
    get:
      responses:
        "200":
          description: list
          content:
            application/json:
              schema: { type: array, items: { $ref: "#/components/schemas/User" } }
  /users/{id}:
    get:
      responses:
        "404": { description: missing }
        "200":
          description: one
          content:
            application/json:
              schema: { $ref: "#/components/schemas/User" }
  /orders:
    post:
      responses:
        "201":
          description: created
          content:
            application/json:
              schema: { type: object, properties: { id: { type: string, format: uuid } } }
    delete:
      responses:
        "204": { description: gone }
components:
  schemas:
    User:
      type: object
      properties:
        id: { type: integer }
        firstName: { type: string }
"##;

fn find<'a>(m: &'a ApiModel, method: &str, path: &str) -> &'a kaanal_data::Endpoint {
    m.endpoints
        .iter()
        .find(|e| e.method == method && e.path == path)
        .expect("endpoint")
}

#[test]
fn loads_endpoints_with_inlined_refs() {
    let m = ApiModel::from_openapi(SPEC).unwrap();
    assert_eq!(m.endpoints.len(), 4);
    let one = find(&m, "GET", "/users/{id}");
    assert_eq!(one.status, 200, "lowest 2xx wins; 404 ignored");
    let schema = one.schema.as_ref().unwrap();
    assert_eq!(schema["properties"]["firstName"]["type"], "string");
    assert!(schema.get("$ref").is_none());
    let list = find(&m, "GET", "/users");
    assert_eq!(
        list.schema.as_ref().unwrap()["items"]["properties"]["id"]["type"],
        "integer"
    );
}

#[test]
fn picks_201_and_no_body_cases() {
    let m = ApiModel::from_openapi(SPEC).unwrap();
    assert_eq!(find(&m, "POST", "/orders").status, 201);
    let del = find(&m, "DELETE", "/orders");
    assert_eq!(del.status, 204);
    assert!(del.schema.is_none());
}

#[test]
fn rejects_openapi_31() {
    let err = ApiModel::from_openapi(&SPEC.replace("3.0.3", "3.1.0")).unwrap_err();
    assert!(err.contains("3.1.0"), "{err}");
}

#[test]
fn rejects_external_ref() {
    let err =
        ApiModel::from_openapi(&SPEC.replace("#/components/schemas/User", "other.yaml#/User"))
            .unwrap_err();
    assert!(err.contains("external $ref"), "{err}");
}

#[test]
fn rejects_recursive_schema() {
    let spec = SPEC.replace(
        "firstName: { type: string }",
        "firstName: { type: string }\n        friend: { $ref: \"#/components/schemas/User\" }",
    );
    let err = ApiModel::from_openapi(&spec).unwrap_err();
    assert!(err.contains("recursive"), "{err}");
}

#[test]
fn rejects_missing_ref_target() {
    let err = ApiModel::from_openapi(
        &SPEC.replace("#/components/schemas/User", "#/components/schemas/Nope"),
    )
    .unwrap_err();
    assert!(err.contains("not found"), "{err}");
}

// Addition (not in plan): a self-referencing schema with fan-out 2 must fail
// immediately, not after 2^depth expansions.
#[test]
fn rejects_branching_recursive_schema_fast() {
    let spec = SPEC.replace(
        "firstName: { type: string }",
        "firstName: { type: string }\n        left: { $ref: \"#/components/schemas/User\" }\n        right: { $ref: \"#/components/schemas/User\" }",
    );
    let err = ApiModel::from_openapi(&spec).unwrap_err();
    assert!(err.contains("recursive"), "{err}");
}

// Addition (not in plan): an acyclic chain where each level references the next
// twice expands to 2^levels nodes ("billion laughs"); it must be capped.
#[test]
fn rejects_ref_expansion_bomb() {
    let levels = 30;
    let mut schemas = String::new();
    for i in 0..levels {
        write!(
            schemas,
            "    S{i}:\n      type: object\n      properties:\n        a: {{ $ref: \"#/components/schemas/S{n}\" }}\n        b: {{ $ref: \"#/components/schemas/S{n}\" }}\n",
            n = i + 1
        )
        .unwrap();
    }
    writeln!(schemas, "    S{levels}: {{ type: string }}").unwrap();
    let spec = format!(
        "openapi: 3.0.3\ninfo: {{ title: t, version: \"1\" }}\npaths:\n  /x:\n    get:\n      responses:\n        \"200\":\n          description: x\n          content:\n            application/json:\n              schema: {{ $ref: \"#/components/schemas/S0\" }}\ncomponents:\n  schemas:\n{schemas}"
    );
    let err = ApiModel::from_openapi(&spec).unwrap_err();
    assert!(err.contains("too many $ref"), "{err}");
}

const REFS: &str = r##"
openapi: 3.0.3
info: { title: t, version: "1" }
paths:
  /chain:
    get: { responses: { "200": { $ref: "#/components/responses/A" } } }
  /alias: { $ref: "#/paths/~1chain" }
components:
  responses:
    A: { $ref: "#/components/responses/B" }
    B:
      description: ok
      content: { application/json: { schema: { type: object, properties: { id: { type: integer } } } } }
"##;

#[test]
fn response_ref_chains_are_followed() {
    let m = ApiModel::from_openapi(REFS).unwrap();
    let chain = find(&m, "GET", "/chain").schema.as_ref().unwrap();
    assert_eq!(chain["properties"]["id"]["type"], "integer");
}

#[test]
fn response_ref_cycle_fails_instead_of_hanging() {
    let spec = REFS.replace(
        "    B:\n      description: ok",
        "    B: { $ref: \"#/components/responses/A\" }\n    C:\n      description: ok",
    );
    let err = ApiModel::from_openapi(&spec).unwrap_err();
    assert!(err.contains("recursive $ref"), "{err}");
}

#[test]
fn path_item_refs_are_followed_or_fail_loudly() {
    let m = ApiModel::from_openapi(REFS).unwrap();
    let alias = find(&m, "GET", "/alias");
    assert_eq!(alias.schema, find(&m, "GET", "/chain").schema);
    let err =
        ApiModel::from_openapi(&REFS.replace("#/paths/~1chain", "other.yaml#/x")).unwrap_err();
    assert!(
        err.contains("/paths/~1alias") && err.contains("external $ref"),
        "{err}"
    );
}

#[test]
fn accepts_unquoted_numeric_version() {
    let spec = "openapi: 3.0
info: { title: t, version: \"1\" }
paths: {}
";
    assert_eq!(ApiModel::from_openapi(spec).unwrap().endpoints, []);
}

#[test]
fn rejects_path_without_leading_slash() {
    let spec = "openapi: 3.0.3
info: { title: t, version: \"1\" }
paths:
  users: { get: { responses: { \"200\": { description: ok } } } }
";
    let err = ApiModel::from_openapi(spec).unwrap_err();
    assert!(err.contains("must start with '/'"), "{err}");
}

#[test]
fn response_example_beats_schema_examples() {
    let spec = r#"
openapi: 3.0.3
info: { title: t, version: "1" }
paths:
  /a:
    get:
      responses:
        "200":
          description: ok
          content:
            application/json:
              example: { n: 1 }
              schema: { type: object, example: { n: 9 }, examples: [{ n: 8 }] }
  /b:
    get:
      responses:
        "200":
          description: ok
          content:
            application/json:
              examples: { one: { value: { n: 2 } } }
              schema: { type: object, example: { n: 9 }, examples: [{ n: 8 }] }
  /c:
    get:
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema: { type: object, examples: [{ n: 8 }] }
"#;
    let m = ApiModel::from_openapi(spec).unwrap();
    let g1 = |p: &str| {
        let s = find(&m, "GET", p).schema.clone().unwrap();
        kaanal_data::generate(&s, None, 1, &kaanal_data::Options::default())
    };
    assert_eq!(g1("/a"), serde_json::json!({ "n": 1 }));
    assert_eq!(g1("/b"), serde_json::json!({ "n": 2 }));
    assert_eq!(g1("/c"), serde_json::json!({ "n": 8 }));
}

#[test]
fn a_dangling_ref_in_media_examples_fails_the_load_with_a_position() {
    let spec = r##"
openapi: 3.0.3
info: { title: t, version: "1" }
paths:
  /a:
    get:
      responses:
        "200":
          description: ok
          content:
            application/json:
              examples: { one: { $ref: "#/components/examples/nope" } }
              schema: { type: object }
"##;
    let err = ApiModel::from_openapi(spec).unwrap_err();
    assert!(
        err.starts_with("/paths/~1a/get") && err.contains("not found"),
        "{err}"
    );
}

/// A one-endpoint spec with the given `servers:` block (YAML text, may be empty).
fn with_servers(servers: &str) -> String {
    format!(
        "openapi: 3.0.3\ninfo: {{ title: t, version: \"1\" }}\n{servers}\npaths:\n  /pets:\n    get:\n      responses:\n        \"200\": {{ description: ok }}\n"
    )
}

fn pet_path(servers: &str) -> String {
    let m = ApiModel::from_openapi(&with_servers(servers)).expect("parses");
    m.endpoints[0].path.clone()
}

#[test]
fn a_relative_server_url_prefixes_every_path() {
    assert_eq!(pet_path("servers: [{ url: /api/v3 }]"), "/api/v3/pets");
}

#[test]
fn an_absolute_server_url_contributes_its_path_only() {
    assert_eq!(
        pet_path("servers: [{ url: \"https://api.example.com:8443/v1/\" }]"),
        "/v1/pets"
    );
}

#[test]
fn no_servers_a_root_url_or_a_host_only_url_add_no_prefix() {
    for servers in [
        "",
        "servers: []",
        "servers: [{ url: / }]",
        "servers: [{ url: \"https://api.example.com\" }]",
        "servers: [{ description: no url }]",
    ] {
        assert_eq!(pet_path(servers), "/pets", "{servers}");
    }
}

#[test]
fn only_the_first_server_counts() {
    assert_eq!(pet_path("servers: [{ url: /a }, { url: /b }]"), "/a/pets");
}

#[test]
fn server_variables_use_their_defaults() {
    let servers = "servers:\n  - url: \"https://{env}.example.com/{ver}\"\n    variables:\n      env: { default: api }\n      ver: { default: v2, enum: [v1, v2] }";
    assert_eq!(pet_path(servers), "/v2/pets");
}

#[test]
fn a_variable_without_a_default_is_an_error() {
    let err = ApiModel::from_openapi(&with_servers("servers: [{ url: \"/{ver}\" }]")).unwrap_err();
    assert!(err.contains("ver") && err.contains("default"), "{err}");
}

#[test]
fn a_prefix_that_is_still_a_template_is_an_error() {
    let servers = "servers:\n  - url: /{p}\n    variables:\n      p: { default: \"{x}\" }";
    let err = ApiModel::from_openapi(&with_servers(servers)).unwrap_err();
    assert!(err.contains("servers"), "{err}");
}
