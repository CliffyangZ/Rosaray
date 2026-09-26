//! Quickstart §5 (US5): offline paper → reviewed method candidates. Uses a
//! fabricated text-layer PDF and a scanned (image-only) PDF; nothing here touches
//! the network, and nothing is ever published or executed.

mod common;

use std::time::Duration;

use reqwest::Method;
use serde_json::{json, Value};

const TEXT_PDF: &[u8] = include_bytes!("fixtures/kb/text-layer.pdf");
const SCANNED_PDF: &[u8] = include_bytes!("fixtures/kb/scanned.pdf");

async fn import(svc: &common::TestService, bytes: &[u8], attested: bool) -> (reqwest::StatusCode, Value) {
    let form = reqwest::multipart::Form::new()
        .part("file", reqwest::multipart::Part::bytes(bytes.to_vec()).file_name("paper.pdf"))
        .text("authorization_attested", attested.to_string())
        .text("title", "Fixture paper");
    let resp = svc.request(Method::POST, "/papers").multipart(form).send().await.unwrap();
    let status = resp.status();
    (status, resp.json().await.unwrap_or(Value::Null))
}

async fn wait_for_state(svc: &common::TestService, paper_id: &str, states: &[&str]) -> Value {
    for _ in 0..200 {
        let (_, body) = svc.json(Method::GET, &format!("/papers/{paper_id}/candidates"), None).await;
        if states.contains(&body["paper"]["extraction_state"].as_str().unwrap_or("")) {
            return body;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("extraction never reached {states:?}");
}

async fn extracted(svc: &common::TestService, bytes: &[u8]) -> (String, Value) {
    let (status, imp) = import(svc, bytes, true).await;
    assert!(status == 201 || status == 200, "{imp}");
    let paper_id = imp["paper_id"].as_str().unwrap().to_string();
    let (status, started) = svc.json(Method::POST, &format!("/papers/{paper_id}/extract"), None).await;
    assert_eq!(status, 202, "{started}");
    (paper_id.clone(), wait_for_state(svc, &paper_id, &["extracted", "cancelled", "failed"]).await)
}

fn find<'a>(candidates: &'a Value, pred: impl Fn(&Value) -> bool) -> &'a Value {
    candidates["candidates"].as_array().unwrap().iter().find(|c| pred(c)).unwrap_or_else(|| panic!("no matching candidate in {candidates}"))
}

fn quote_has(c: &Value, needle: &str) -> bool {
    c["sources"].as_array().unwrap().iter().any(|s| s["quote"].as_str().unwrap().contains(needle))
}

/// Every proposed value is anchored in its quote — no defaulted value anywhere (SC-007).
fn assert_nothing_invented(candidates: &Value) {
    let items = |c: &Value| -> Vec<Value> {
        let p = &c["proposed"];
        let mut v = Vec::new();
        for k in ["inputs", "outputs", "parameters", "units", "assumptions"] {
            v.extend(p[k].as_array().cloned().unwrap_or_default());
        }
        if !p["formula"].is_null() {
            v.push(p["formula"].clone());
        }
        v
    };
    for c in candidates["candidates"].as_array().unwrap() {
        for it in items(c) {
            if !it["value"].is_null() || !it["unit"].is_null() {
                let span = &it["source_span"];
                assert!(!span.is_null(), "unanchored value/unit in {it}");
                let quote = span["quote"].as_str().unwrap().to_ascii_lowercase();
                if let Some(u) = it["unit"].as_str() {
                    assert!(quote.contains(&u.to_ascii_lowercase()), "unit {u} not in its quote: {quote}");
                }
                match &it["value"] {
                    Value::Number(n) => {
                        let f = n.as_f64().unwrap();
                        let text = if f.fract() == 0.0 { format!("{}", f as i64) } else { format!("{f}") };
                        assert!(quote.contains(&text), "number {text} not in its quote: {quote}");
                    }
                    Value::String(s) => {
                        let head: String = s.split_whitespace().take(3).collect::<Vec<_>>().join(" ").to_ascii_lowercase();
                        assert!(quote.contains(&head), "value {s} not in its quote: {quote}");
                    }
                    _ => {}
                }
            }
        }
    }
}

#[tokio::test]
async fn import_needs_attestation_a_pdf_and_is_idempotent() {
    let svc = common::spawn().await;
    let (status, body) = import(&svc, TEXT_PDF, false).await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["error"]["code"], "authorization_required");
    let (status, body) = import(&svc, b"not a pdf at all", true).await;
    assert_eq!(status, 422);
    assert_eq!(body["error"]["code"], "bundle_invalid");
    let (status, first) = import(&svc, TEXT_PDF, true).await;
    assert_eq!(status, 201, "{first}");
    assert_eq!(first["page_count"], 2);
    assert_eq!(first["pages_without_text"], json!([]));
    let (status, again) = import(&svc, TEXT_PDF, true).await;
    assert_eq!(status, 200);
    assert_eq!(again["paper_id"], first["paper_id"]);
    assert_eq!(again["already_imported"], true);

    // Importing extracts nothing, publishes nothing and touches no bundle.
    let (_, cands) = svc.json(Method::GET, &format!("/papers/{}/candidates", first["paper_id"].as_str().unwrap()), None).await;
    assert_eq!(cands["paper"]["extraction_state"], "imported");
    assert!(cands["candidates"].as_array().unwrap().is_empty());
    assert!(svc.entries("").await.is_empty());

    // The paper is stored encrypted: its bytes are not recoverable from the blob directory.
    let blobs = svc._project_dir_path().join("blobs");
    for entry in walk(&blobs) {
        let bytes = std::fs::read(entry).unwrap();
        assert!(!bytes.windows(9).any(|w| w == b"Gingival "), "plaintext paper text on disk");
    }
    // The audit trail names no file and no text.
    let db = svc.state.db.lock().unwrap();
    let detail: String = db.query_row("SELECT detail_json FROM kb_event WHERE type = 'paper_import'", [], |r| r.get(0)).unwrap();
    assert!(!detail.contains("paper.pdf") && !detail.contains("Gingival"), "{detail}");
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.path().is_dir() {
                out.extend(walk(&e.path()));
            } else {
                out.push(e.path());
            }
        }
    }
    out
}

#[tokio::test]
async fn candidates_carry_page_quote_category_and_never_invented_values() {
    let svc = common::spawn().await;
    let (_, body) = extracted(&svc, TEXT_PDF).await;
    assert_eq!(body["paper"]["extraction_state"], "extracted");
    let cats: std::collections::BTreeSet<_> = body["candidates"].as_array().unwrap().iter().map(|c| c["category"].as_str().unwrap()).collect();
    for want in ["data_eligibility", "preprocessing", "measurement", "validation_only", "clinical_claim"] {
        assert!(cats.contains(want), "{want} missing from {cats:?}");
    }
    for c in body["candidates"].as_array().unwrap() {
        assert_eq!(c["origin"], "paper_derived");
        assert_eq!(c["state"], "proposed");
        assert!(!c["sources"].as_array().unwrap().is_empty());
        for s in c["sources"].as_array().unwrap() {
            assert!(s["page"].as_u64().unwrap() >= 1);
            assert!(!s["quote"].as_str().unwrap().is_empty());
        }
    }
    // The formula is proposed only because the paper writes it out.
    let formula = find(&body, |c| c["proposed"]["formula"].is_object());
    assert_eq!(formula["proposed"]["formula"]["value"], "A = N x s^2");
    // The area sentence describes a computation in words: an ambiguity, not an invented formula.
    let area = find(&body, |c| quote_has(c, "pixel count multiplied"));
    assert!(area["proposed"]["formula"].is_null());
    assert!(area["ambiguities"].as_array().unwrap().iter().any(|a| a["code"] == "formula_unparsed"));
    // Unit present ⇒ anchored; the pixel spacing carries its mm.
    let spacing = area["proposed"]["parameters"].as_array().unwrap().iter().find(|p| p["name"] == "pixel spacing").unwrap();
    assert_eq!((spacing["value"].clone(), spacing["unit"].clone()), (json!(0.05), json!("mm")));

    assert_nothing_invented(&body);
}

#[tokio::test]
async fn a_missing_unit_is_null_plus_an_ambiguity_never_a_default() {
    let svc = common::spawn().await;
    let (_, body) = extracted(&svc, TEXT_PDF).await;
    let gauss = find(&body, |c| quote_has(c, "Gaussian"));
    let sigma = gauss["proposed"]["parameters"].as_array().unwrap().iter().find(|p| p["name"] == "sigma").unwrap();
    assert_eq!(sigma["value"], 2.0);
    assert!(sigma["unit"].is_null(), "the source gives no unit for sigma");
    assert!(gauss["ambiguities"].as_array().unwrap().iter().any(|a| a["code"] == "missing_unit" && a["field"] == "sigma"));
    let kept = find(&body, |c| quote_has(c, "Pixels above 0.6"));
    assert!(kept["proposed"]["parameters"][0]["unit"].is_null());
    assert!(kept["ambiguities"].as_array().unwrap().iter().any(|a| a["code"] == "missing_unit" && a["field"] == "threshold"));
    // Serialized form of the whole result contains no "unit" the sources do not state.
    assert_nothing_invented(&body);
}

#[tokio::test]
async fn two_passages_about_one_step_stay_together_through_review() {
    let svc = common::spawn().await;
    let (_, body) = extracted(&svc, TEXT_PDF).await;
    let gauss = find(&body, |c| quote_has(c, "Gaussian"));
    let pages: Vec<_> = gauss["sources"].as_array().unwrap().iter().map(|s| s["page"].as_u64().unwrap()).collect();
    assert_eq!(pages, vec![1, 2], "both supporting passages are sources of one candidate");
    let id = gauss["candidate_id"].as_str().unwrap();

    let (status, out) = svc.json(Method::POST, &format!("/candidates/{id}/decisions"), Some(json!({ "action": "accept" }))).await;
    assert_eq!(status, 201, "{out}");
    assert_eq!(out["candidate"]["state"], "accepted");
    assert_eq!(out["candidate"]["sources"].as_array().unwrap().len(), 2, "all evidence references remain associated after save");
    let draft_id = out["draft"]["id"].as_str().unwrap();

    // One method-source evidence record per source, all non-distributable and patient-free.
    let refs: Vec<_> = walk(&svc.kb_root.join("drafts/nodes").join(draft_id).join("references"));
    assert_eq!(refs.len(), 2);
    let text = std::fs::read_to_string(&refs[0]).unwrap();
    assert!(text.contains("method_source") && text.contains("distributable: false"), "{text}");
    assert!(text.contains("paper_source_hash"));
}

#[tokio::test]
async fn accepting_makes_a_specification_only_paper_derived_draft_that_cannot_preview() {
    let svc = common::spawn().await;
    svc.install_seeds();
    let (_, body) = extracted(&svc, TEXT_PDF).await;
    let gauss = find(&body, |c| quote_has(c, "Gaussian"));
    let (status, out) = svc
        .json(Method::POST, &format!("/candidates/{}/decisions", gauss["candidate_id"].as_str().unwrap()), Some(json!({ "action": "accept", "rationale": "matches our protocol" })))
        .await;
    assert_eq!(status, 201, "{out}");
    let draft_id = out["draft"]["id"].as_str().unwrap().to_string();
    assert!(draft_id.starts_with("paper.preprocessing-"));

    let entry = svc.entries("?kind=algonode&release=draft").await.into_iter().find(|e| e["id"] == draft_id.as_str()).unwrap();
    assert_eq!(entry["maturity"], "specification_only");
    assert_eq!(entry["origin"], "paper_derived", "still distinguishable from authored nodes");
    assert_eq!(entry["status"], "draft");

    // The extracted value is a *default with a citation*, not an unsourced one.
    let contract = std::fs::read_to_string(svc.kb_root.join("drafts/nodes").join(&draft_id).join("contract.yaml")).unwrap();
    let doc: Value = serde_yaml_ng::from_str(&contract).unwrap();
    let sigma = doc["parameters"].as_array().unwrap().iter().find(|p| p["parameter_id"] == "sigma").unwrap();
    assert_eq!(sigma["default"], 2.0);
    assert_eq!(sigma["source"], "paper");
    assert_eq!(sigma["evidence_ref"], "paper-1");
    assert!(sigma.get("unit").is_none(), "no unit was invented");
    let (_, draft) = svc.json(Method::GET, &format!("/kb/algonode/{draft_id}/draft"), None).await;
    assert!(draft["files"]["ALGONODE.md"].as_str().unwrap().contains("origin: paper_derived"));
    // The draft is honest about what the paper did not say: nothing is defaulted, so it reports gaps.
    assert!(draft["findings"].as_array().unwrap().iter().any(|f| f["code"] == "reproducibility_undeclared"), "{}", draft["findings"]);

    // Nothing was published, and Preview is blocked (specification only).
    assert!(svc.entries("?status=published&release=knowledge").await.iter().all(|e| e["id"].as_str().unwrap().starts_with("rosaray.")));
    let (_, created) = svc.json(Method::POST, "/kb/algopipe/drafts", Some(json!({ "id": "acme.paperpipe" }))).await;
    let graph = json!({ "schema": "quantify-kb/1", "nodes": [{ "instance_id": "p", "ref": { "id": draft_id, "version": "draft" } }], "edges": [] });
    let (status, saved) = svc.json(Method::PUT, "/kb/algopipe/drafts/acme.paperpipe", Some(json!({ "base_revision": created["revision"], "graph": graph }))).await;
    assert_eq!(status, 200, "{saved}");
    let dir = tempfile::tempdir().unwrap();
    common::write_png(&dir.path().join("a.png"), 90);
    let (_, version) = svc.import_dir(dir.path(), "DS").await;
    let image = svc.image_ids(&version).await.remove(0);
    let (status, blocked) = svc
        .json(Method::POST, "/preview", Some(json!({ "pipe_id": "acme.paperpipe", "revision": saved["revision"], "image_asset_id": image, "target_node_id": "p" })))
        .await;
    assert_eq!(status, 409, "{blocked}");
    assert_eq!(blocked["error"]["code"], "not_previewable");

    // It cannot be published as executable; knowledge-only remains possible once complete.
    let (status, body) = svc
        .json(Method::POST, &format!("/kb/algonode/drafts/{draft_id}/publish"), Some(json!({ "release": "executable", "base_revision": draft["revision"] })))
        .await;
    assert_eq!(status, 422, "{body}");

    // Re-deciding updates the same draft (and refuses to clobber a researcher's edits).
    let (status, again) = svc
        .json(Method::POST, &format!("/candidates/{}/decisions", gauss["candidate_id"].as_str().unwrap()), Some(json!({ "action": "keep_specification_only" })))
        .await;
    assert_eq!(status, 200, "{again}");
    assert_eq!(again["draft"]["id"], draft_id.as_str());
    let contract_path = svc.kb_root.join("drafts/nodes").join(&draft_id).join("contract.yaml");
    std::fs::write(&contract_path, format!("{}# researcher note\n", std::fs::read_to_string(&contract_path).unwrap())).unwrap();
    let (status, conflict) = svc
        .json(Method::POST, &format!("/candidates/{}/decisions", gauss["candidate_id"].as_str().unwrap()), Some(json!({ "action": "accept" })))
        .await;
    assert_eq!(status, 409, "{conflict}");
    assert_eq!(conflict["error"]["code"], "draft_conflict");
    assert!(std::fs::read_to_string(&contract_path).unwrap().contains("researcher note"));
}

#[tokio::test]
async fn every_action_is_available_and_history_is_append_only() {
    let svc = common::spawn().await;
    svc.install_seeds();
    let (_, body) = extracted(&svc, TEXT_PDF).await;
    let cands: Vec<Value> = body["candidates"].as_array().unwrap().clone();
    let pick = |needle: &str| cands.iter().find(|c| quote_has(c, needle)).unwrap().clone();
    let post = |id: String, b: Value| {
        let svc = &svc;
        async move { svc.json(Method::POST, &format!("/candidates/{id}/decisions"), Some(b)).await }
    };

    // reject / non-executable / defer
    let elig = pick("motion blur");
    let (s, out) = post(elig["candidate_id"].as_str().unwrap().into(), json!({ "action": "reject", "rationale": "out of scope" })).await;
    assert_eq!(s, 200, "{out}");
    assert_eq!(out["candidate"]["state"], "rejected");
    let valid = pick("Dice coefficient");
    let (_, out) = post(valid["candidate_id"].as_str().unwrap().into(), json!({ "action": "mark_non_executable" })).await;
    assert_eq!(out["candidate"]["state"], "non_executable");
    let post_c = pick("Morphological opening");
    let (_, out) = post(post_c["candidate_id"].as_str().unwrap().into(), json!({ "action": "defer" })).await;
    assert_eq!(out["candidate"]["state"], "deferred");
    // A deferred candidate can be decided again; both decisions stay on record.
    let id = post_c["candidate_id"].as_str().unwrap().to_string();
    let (s, out) = post(id.clone(), json!({ "action": "map_to_node", "map_to": { "id": "rosaray.morphology", "version": "1.0.0" } })).await;
    assert_eq!(s, 200, "{out}");
    assert_eq!(out["candidate"]["state"], "mapped");
    assert_eq!(out["decision"]["target_ref"], "rosaray.morphology@1.0.0");
    let (_, hist) = svc.json(Method::GET, &format!("/candidates/{id}/decisions"), None).await;
    let actions: Vec<_> = hist["decisions"].as_array().unwrap().iter().map(|d| d["action"].as_str().unwrap()).collect();
    assert_eq!(actions, vec!["defer", "map_to_node"]);

    // edit stores the after-snapshot; incompatible mappings return findings and change nothing.
    let (s, out) = post(
        id.clone(),
        json!({ "action": "edit", "edited": { "proposed": { "parameters": [{ "name": "radius", "value": 3, "unit": "mm" }], "inputs": [{ "name": "image" }] } } }),
    )
    .await;
    assert_eq!(s, 200, "{out}");
    assert_eq!(out["candidate"]["state"], "edited");
    assert_eq!(out["decision"]["edited_snapshot"]["proposed"]["parameters"][0]["unit"], "mm");
    let (s, out) = post(id.clone(), json!({ "action": "map_to_node", "map_to": { "id": "rosaray.morphology", "version": "1.0.0" } })).await;
    assert_eq!(s, 422, "{out}");
    let codes: Vec<_> = out["error"]["details"]["findings"].as_array().unwrap().iter().map(|f| f["code"].as_str().unwrap()).collect();
    assert!(codes.contains(&"port_incompatible.unit") && codes.contains(&"port_incompatible.type"), "{codes:?}");
    for f in out["error"]["details"]["findings"].as_array().unwrap() {
        assert!(!f["explanation"].as_str().unwrap().is_empty() && !f["action"].as_str().unwrap().is_empty());
    }
    let (_, after) = svc.json(Method::GET, &format!("/papers/{}/candidates?state=edited", body["paper"]["paper_id"].as_str().unwrap()), None).await;
    assert_eq!(after["candidates"].as_array().unwrap().len(), 1, "a refused mapping changes nothing");
    let (s, _) = post(id.clone(), json!({ "action": "map_to_node", "map_to": { "id": "rosaray.nope", "version": "1.0.0" } })).await;
    assert_eq!(s, 404);
    let (s, _) = post(id.clone(), json!({ "action": "frobnicate" })).await;
    assert_eq!(s, 422);

    // Decisions are append-only at the database level, too.
    let db = svc.state.db.lock().unwrap();
    assert!(db.execute("UPDATE review_decision SET action = 'accept'", []).is_err());
    assert!(db.execute("DELETE FROM review_decision", []).is_err());
    let n: i64 = db.query_row("SELECT count(*) FROM kb_event WHERE type = 'candidate_decision'", [], |r| r.get(0)).unwrap();
    assert!(n >= 5);
    // Nothing the review did published anything.
    assert!(db.query_row("SELECT count(*) FROM kb_catalog_entry WHERE status = 'published' AND id NOT LIKE 'rosaray.%'", [], |r| r.get::<_, i64>(0)).unwrap() == 0);
}

#[tokio::test]
async fn a_clinical_claim_can_only_be_rejected_or_marked_non_executable() {
    let svc = common::spawn().await;
    let (_, body) = extracted(&svc, TEXT_PDF).await;
    let claim = find(&body, |c| c["category"] == "clinical_claim");
    assert!(quote_has(claim, "diagnoses periodontitis"));
    assert!(claim["proposed"]["parameters"].as_array().unwrap().is_empty(), "no method is extracted from a clinical claim");
    let id = claim["candidate_id"].as_str().unwrap();
    for action in ["accept", "keep_specification_only"] {
        let (s, out) = svc.json(Method::POST, &format!("/candidates/{id}/decisions"), Some(json!({ "action": action }))).await;
        assert_eq!(s, 422, "{action}: {out}");
        assert_eq!(out["error"]["code"], "clinical_claim_not_executable");
    }
    let (s, out) = svc
        .json(Method::POST, &format!("/candidates/{id}/decisions"), Some(json!({ "action": "map_to_node", "map_to": { "id": "x.y", "version": "1.0.0" } })))
        .await;
    assert_eq!(s, 422, "{out}");
    let (s, _) = svc.json(Method::POST, &format!("/candidates/{id}/decisions"), Some(json!({ "action": "edit", "edited": { "category": "measurement" } }))).await;
    assert_eq!(s, 422, "editing the category cannot launder a clinical claim");
    assert!(svc.entries("").await.is_empty(), "no draft was made");
    for action in ["mark_non_executable", "reject"] {
        let (s, out) = svc.json(Method::POST, &format!("/candidates/{id}/decisions"), Some(json!({ "action": action }))).await;
        assert_eq!(s, 200, "{action}: {out}");
    }
}

#[tokio::test]
async fn a_scanned_pdf_reports_pages_without_text_and_fabricates_nothing() {
    let svc = common::spawn().await;
    let mut events = svc.state.event_tx.subscribe();
    let (status, imp) = import(&svc, SCANNED_PDF, true).await;
    assert_eq!(status, 201);
    assert_eq!(imp["pages_without_text"], json!([1, 2]));
    let paper_id = imp["paper_id"].as_str().unwrap().to_string();
    svc.json(Method::POST, &format!("/papers/{paper_id}/extract"), None).await;
    let body = wait_for_state(&svc, &paper_id, &["extracted", "failed"]).await;
    assert_eq!(body["paper"]["extraction_state"], "extracted");
    assert!(body["candidates"].as_array().unwrap().is_empty(), "zero fabricated candidates");
    assert_eq!(body["paper"]["pages_without_text"], json!([1, 2]));
    let complete = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let v = serde_json::to_value(&events.recv().await.unwrap()).unwrap();
            if v["type"] == "extraction_complete" {
                return v;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(complete["payload"]["candidate_count"], 0);
    assert_eq!(complete["payload"]["pages_without_text"], json!([1, 2]));
}

#[tokio::test]
async fn cancelling_an_extraction_commits_nothing_and_changes_no_catalog() {
    let svc = common::spawn().await;
    svc.install_seeds();
    let (_, imp) = import(&svc, TEXT_PDF, true).await;
    let paper_id = imp["paper_id"].as_str().unwrap().to_string();
    svc.state.extraction_delay_ms.store(400, std::sync::atomic::Ordering::SeqCst);
    let before_tree = common::hash_tree(&svc.kb_root);
    let before_entries = svc.entries("").await;
    let mut events = svc.state.event_tx.subscribe();

    let (_, started) = svc.json(Method::POST, &format!("/papers/{paper_id}/extract"), None).await;
    let extraction_id = started["extraction_id"].as_str().unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let (status, _) = svc.json(Method::DELETE, &format!("/papers/{paper_id}/extract/{extraction_id}"), None).await;
    assert_eq!(status, 204);

    let body = wait_for_state(&svc, &paper_id, &["cancelled", "extracted", "failed"]).await;
    assert_eq!(body["paper"]["extraction_state"], "cancelled");
    assert!(body["candidates"].as_array().unwrap().is_empty(), "no candidates committed");
    let seen = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let v = serde_json::to_value(&events.recv().await.unwrap()).unwrap();
            if v["type"] == "extraction_cancelled" {
                return v;
            }
        }
    })
    .await
    .expect("extraction_cancelled");
    assert_eq!(seen["payload"]["extraction_id"], extraction_id);
    assert_eq!(common::hash_tree(&svc.kb_root), before_tree, "published set unchanged");
    assert_eq!(svc.entries("").await, before_entries, "catalog unchanged");

    // A later, uncancelled extraction of the same paper works normally.
    svc.state.extraction_delay_ms.store(0, std::sync::atomic::Ordering::SeqCst);
    svc.json(Method::POST, &format!("/papers/{paper_id}/extract"), None).await;
    let body = wait_for_state(&svc, &paper_id, &["extracted", "failed"]).await;
    assert_eq!(body["paper"]["extraction_state"], "extracted");
    assert!(!body["candidates"].as_array().unwrap().is_empty());
    // Cancelling something already finished is not an error.
    let (status, _) = svc.json(Method::DELETE, &format!("/papers/{paper_id}/extract/{}", uuid::Uuid::new_v4()), None).await;
    assert_eq!(status, 204);
}
