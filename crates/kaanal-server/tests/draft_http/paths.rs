//! A draft comes through the API, so the files it names must stay inside the project folder, and
//! what comes back must not show the machine's layout.

use crate::common::*;
use reqwest::Method;
use serde_json::{Value, json};

async fn validate(s: &Srv, text: &str) -> Value {
    assert_eq!(
        s.put("/__kaanal/draft", json!({ "text": text }))
            .await
            .status(),
        200
    );
    let r = s.send(Method::POST, "/__kaanal/draft/validate", None).await;
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

fn with_spec(spec: &str) -> String {
    system("1ms").replace("spec: openapi.yaml", &format!("spec: '{spec}'"))
}

fn message(v: &Value) -> String {
    assert_eq!(v["ok"], false, "{v}");
    v["diagnostics"][0]["message"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn an_absolute_spec_path_is_refused_even_when_the_file_exists() {
    let s = start(&system("1ms")).await;
    let abs = s.dir.join("openapi.yaml");
    assert!(
        abs.is_file(),
        "the file is there, so the refusal is not a missing file"
    );
    let v = validate(&s, &with_spec(&abs.display().to_string())).await;
    let m = message(&v);
    assert!(m.contains("inside the project folder"), "{m}");
    assert_eq!(v["diagnostics"][0]["path"], json!(["nodes", "app", "spec"]));
    assert!(
        !m.contains(&s.dir.display().to_string()),
        "no absolute path in {m}"
    );
}

#[tokio::test]
async fn a_parent_dir_spec_path_is_refused() {
    let s = start(&system("1ms")).await;
    let v = validate(&s, &with_spec("../openapi.yaml")).await;
    assert!(message(&v).contains("inside the project folder"), "{v}");
    // also through a subfolder
    let v = validate(&s, &with_spec("specs/../../x.yaml")).await;
    assert!(message(&v).contains("inside the project folder"), "{v}");
}

#[tokio::test]
async fn data_and_endpoints_files_are_held_to_the_project_too() {
    let s = start(&system("1ms")).await;
    let abs = s.dir.join("openapi.yaml").display().to_string();
    let v = validate(&s, &format!("{}data: '{abs}'\n", system("1ms"))).await;
    let m = message(&v);
    assert!(m.contains("inside the project folder"), "{m}");
    assert_eq!(v["diagnostics"][0]["path"], json!(["data"]));
    let v = validate(&s, &format!("{}endpoints: '../x.yaml'\n", system("1ms"))).await;
    assert!(message(&v).contains("inside the project folder"), "{v}");
}

#[tokio::test]
async fn a_missing_or_non_file_spec_reports_a_relative_path_and_no_content() {
    let s = start(&system("1ms")).await;
    let here = s.dir.display().to_string();
    let v = validate(&s, &with_spec("nope.yaml")).await;
    let m = message(&v);
    assert!(m.contains("nope.yaml") && !m.contains(&here), "{m}");
    // a folder is not a spec, and says so instead of being read
    std::fs::create_dir_all(s.dir.join("specs")).unwrap();
    let m = message(&validate(&s, &with_spec("specs")).await);
    assert!(
        m.contains("not a regular file") && !m.contains(&here),
        "{m}"
    );
}

#[tokio::test]
async fn a_spec_in_a_subfolder_of_the_project_is_fine_and_apply_refuses_the_rest() {
    let s = start(&system("1ms")).await;
    std::fs::create_dir_all(s.dir.join("specs")).unwrap();
    std::fs::copy(s.dir.join("openapi.yaml"), s.dir.join("specs/api.yaml")).unwrap();
    let v = validate(&s, &with_spec("specs/api.yaml")).await;
    assert_eq!(v["ok"], true, "{v}");
    // Apply holds the same line
    let bad = with_spec("../openapi.yaml");
    assert_eq!(
        s.put("/__kaanal/draft", json!({ "text": bad }))
            .await
            .status(),
        200
    );
    let r = s.send(Method::POST, "/__kaanal/draft/apply", None).await;
    assert_eq!(r.status(), 400);
    let v: Value = r.json().await.unwrap();
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains("inside the project folder"),
        "{v}"
    );
}
