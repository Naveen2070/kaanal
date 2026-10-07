use kaanal_data::ApiModel;

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
    assert!(err.to_string().contains("3.1.0"), "{err}");
}

#[test]
fn rejects_external_ref() {
    let err =
        ApiModel::from_openapi(&SPEC.replace("#/components/schemas/User", "other.yaml#/User"))
            .unwrap_err();
    assert!(err.to_string().contains("external $ref"), "{err}");
}

#[test]
fn rejects_recursive_schema() {
    let spec = SPEC.replace(
        "firstName: { type: string }",
        "firstName: { type: string }\n        friend: { $ref: \"#/components/schemas/User\" }",
    );
    let err = ApiModel::from_openapi(&spec).unwrap_err();
    assert!(err.to_string().contains("recursive"), "{err}");
}

#[test]
fn rejects_missing_ref_target() {
    let err = ApiModel::from_openapi(
        &SPEC.replace("#/components/schemas/User", "#/components/schemas/Nope"),
    )
    .unwrap_err();
    assert!(err.to_string().contains("not found"), "{err}");
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
    assert!(err.to_string().contains("recursive"), "{err}");
}

// Addition (not in plan): an acyclic chain where each level references the next
// twice expands to 2^levels nodes ("billion laughs"); it must be capped.
#[test]
fn rejects_ref_expansion_bomb() {
    let levels = 30;
    let mut schemas = String::new();
    for i in 0..levels {
        schemas.push_str(&format!(
            "    S{i}:\n      type: object\n      properties:\n        a: {{ $ref: \"#/components/schemas/S{n}\" }}\n        b: {{ $ref: \"#/components/schemas/S{n}\" }}\n",
            n = i + 1
        ));
    }
    schemas.push_str(&format!("    S{levels}: {{ type: string }}\n"));
    let spec = format!(
        "openapi: 3.0.3\ninfo: {{ title: t, version: \"1\" }}\npaths:\n  /x:\n    get:\n      responses:\n        \"200\":\n          description: x\n          content:\n            application/json:\n              schema: {{ $ref: \"#/components/schemas/S0\" }}\ncomponents:\n  schemas:\n{schemas}"
    );
    let err = ApiModel::from_openapi(&spec).unwrap_err();
    assert!(err.to_string().contains("too many $ref"), "{err}");
}

#[test]
fn accepts_unquoted_numeric_version() {
    let spec = "openapi: 3.0
info: { title: t, version: \"1\" }
paths: {}
";
    assert!(ApiModel::from_openapi(spec).unwrap().endpoints.is_empty());
}

#[test]
fn rejects_path_without_leading_slash() {
    let spec = "openapi: 3.0.3
info: { title: t, version: \"1\" }
paths:
  users: { get: { responses: { \"200\": { description: ok } } } }
";
    let err = ApiModel::from_openapi(spec).unwrap_err();
    assert!(err.to_string().contains("must start with '/'"), "{err}");
}
