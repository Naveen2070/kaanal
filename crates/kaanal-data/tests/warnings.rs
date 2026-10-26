use kaanal_data::ApiModel;

#[test]
fn api_model_collects_warnings_per_endpoint() {
    let spec = r#"
openapi: 3.0.3
info: { title: t, version: "1" }
paths:
  /things:
    get:
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema: { type: object, properties: { code: { type: string, pattern: "^A" } } }
"#;
    let w = ApiModel::from_openapi(spec).unwrap().warnings();
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].starts_with("GET /things"), "{w:?}");
}

fn paged(props: &str) -> String {
    format!(
        r#"
openapi: 3.0.3
info: {{ title: t, version: "1" }}
paths:
  /things:
    get:
      parameters:
        - {{ name: page, in: query, schema: {{ type: integer }} }}
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema: {{ type: object, properties: {props} }}
"#
    )
}

#[test]
fn two_array_properties_warn_once() {
    let arr = "{ type: array, items: { type: string } }";
    let w = ApiModel::from_openapi(&paged(&format!(
        "{{ zeta: {arr}, alpha: {arr}, beta: {arr} }}"
    )))
    .unwrap()
    .warnings();
    assert_eq!(
        w,
        vec![
            "GET /things: response has 3 array properties; paging uses 'alpha' (the first in key order)"
                .to_string()
        ]
    );
    // a named property wins and is said so
    let w = ApiModel::from_openapi(&paged(&format!("{{ alpha: {arr}, data: {arr} }}")))
        .unwrap()
        .warnings();
    assert_eq!(
        w,
        vec![
            "GET /things: response has 2 array properties; paging uses 'data' (a list name wins over key order)"
                .to_string()
        ]
    );
    // one array property, or an unpaged route: no warning
    let one = ApiModel::from_openapi(&paged(&format!("{{ alpha: {arr} }}"))).unwrap();
    assert_eq!(one.warnings(), Vec::<String>::new());
    let unpaged =
        paged(&format!("{{ alpha: {arr}, beta: {arr} }}")).replace("name: page", "name: q");
    assert_eq!(
        ApiModel::from_openapi(&unpaged).unwrap().warnings(),
        Vec::<String>::new()
    );
}
