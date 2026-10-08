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
