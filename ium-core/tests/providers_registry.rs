//! The provider registry × the engine's own parser (the destination lane,
//! owner-ratified 2026-11-11: "regular users cannot get those direct
//! destination urls… they usually only go to a website X").
//!
//! RED artifact: at the pre-implementation HEAD this file FAILS — the
//! registry JSON does not exist yet. On the implementation commit it goes
//! GREEN and stays as the permanent drift guard: the app's provider
//! presets can NEVER claim an endpoint the engine's `parse_target`
//! refuses, because every preset's sample URL is re-fed through the REAL
//! parser here. One JSON file is the single source of truth; both the TS
//! loader and this test consume it — divergence is how docs rot.

use std::path::PathBuf;

use serde_json::Value;

use ium_core::adapter_s3::parse_target;

fn registry_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("ium-app/src/data/providers.json")
}

fn registry() -> Value {
    let path = registry_path();
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("registry unreadable at {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("registry is valid JSON")
}

fn providers() -> Vec<Value> {
    registry()
        .get("providers")
        .and_then(|p| p.as_array())
        .cloned()
        .expect("registry carries a providers array")
}

/// The endpoint template with every slot substituted from the preset's own
/// sample values — rebuilt here so the sample can never drift from the
/// template (a hand-maintained sample would rot the first time someone
/// edits a template without re-copying the example).
fn filled_template(p: &Value) -> String {
    let template = p["endpoint_template"].as_str().expect("template");
    let values = p["sample_values"].as_object().expect("sample values");
    let mut out = template.to_string();
    for (k, v) in values {
        let slot = format!("{{{k}}}");
        assert!(
            out.contains(&slot),
            "preset {}: sample value '{k}' has no slot in its template",
            p["id"]
        );
        out = out.replace(&slot, v.as_str().expect("slot values are strings"));
    }
    out
}

#[test]
fn registry_exists_with_enough_providers() {
    let list = providers();
    assert!(
        list.len() >= 6,
        "the launch set promised at least six S3-compatible presets, got {}",
        list.len()
    );
    assert_eq!(
        registry().get("version").and_then(|v| v.as_i64()),
        Some(1),
        "registry schema version"
    );
}

#[test]
fn preset_ids_are_unique_and_snake_cased() {
    let mut seen = Vec::new();
    for p in providers() {
        let id = p["id"].as_str().expect("id").to_string();
        assert!(!id.is_empty(), "empty id");
        assert!(
            id.chars()
                .all(|c| c.is_ascii_lowercase() || c == '-' || c.is_ascii_digit()),
            "id '{id}' must be lowercase kebab (it rides UI state)"
        );
        assert!(!seen.contains(&id), "duplicate preset id '{id}'");
        seen.push(id);
    }
}

/// THE drift guard: every preset's sample URL must parse with the REAL
/// engine parser — the app can never offer a destination the engine
/// refuses. `parse_target` demands `http(s)://host/bucket[/key]` (the
/// path-style contract, adapter_s3.rs) and derives a missing key from the
/// source basename, so bucket-only samples are legal.
#[test]
fn every_sample_url_parses_with_the_engine() {
    for p in providers() {
        let url = p["sample_url"].as_str().expect("sample_url");
        let parsed = parse_target(url, "/tmp/ium-sample.tar").unwrap_or_else(|e| {
            panic!(
                "preset {}: sample_url '{url}' refused by the engine: {e}",
                p["id"]
            )
        });
        assert!(
            !parsed.bucket.is_empty(),
            "preset {}: empty bucket",
            p["id"]
        );
        assert!(!parsed.host.is_empty(), "preset {}: empty host", p["id"]);
    }
}

/// Template ↔ sample coherence, byte-exact: filled_template + the sample
/// bucket + the sample key must reproduce sample_url. Editing a template
/// without re-deriving the sample breaks this test, never a user.
#[test]
fn sample_urls_match_their_templates_byte_exact() {
    for p in providers() {
        let base = filled_template(&p);
        let bucket = p["sample_bucket"].as_str().expect("sample_bucket");
        let key = p["sample_key"].as_str().expect("sample_key");
        let expected = format!("{base}/{bucket}/{key}");
        assert_eq!(
            expected,
            p["sample_url"].as_str().expect("sample_url"),
            "preset {}: sample_url does not match its filled template",
            p["id"]
        );
    }
}

/// V1 scope: every preset rides the s3_multipart lane (bridge lanes own
/// their endpoint shapes — `rclone-bridge://<key>` — and are not presetted
/// yet). The TS loader enforces the same; this is the engine-side pin.
#[test]
fn v1_scope_all_presets_ride_s3_multipart() {
    for p in providers() {
        assert_eq!(
            p["adapter"].as_str(),
            Some("s3_multipart"),
            "preset {}: only s3_multipart presets ship in v1",
            p["id"]
        );
    }
}

/// Guidance completeness: every preset tells the user where to click for
/// keys (an https page — except self-hosted, honestly null) and carries a
/// non-empty recipe.
#[test]
fn every_preset_carries_key_guidance() {
    for p in providers() {
        let id = p["id"].as_str().expect("id");
        let steps = p["key_steps"].as_str().unwrap_or_default();
        assert!(
            steps.split_whitespace().count() >= 5,
            "preset {id}: key_steps must be a real recipe, got '{steps}'"
        );
        match p["key_page"].as_str() {
            None | Some("") => {
                assert!(
                    p.get("self_hosted")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                    "preset {id}: null key_page is only honest for self-hosted presets"
                );
            }
            Some(url) => {
                assert!(
                    url.starts_with("https://"),
                    "preset {id}: key_page must be https, got '{url}'"
                );
            }
        }
    }
}

/// Region presets carry a default the form can preselect, and every
/// region the template's {region} slot accepts is listed.
#[test]
fn enum_slots_carry_defaults() {
    for p in providers() {
        let template = p["endpoint_template"].as_str().expect("template");
        if !template.contains("{region}") {
            continue;
        }
        let regions = p["regions"].as_array().expect("regions list");
        assert!(!regions.is_empty(), "preset {}: empty region list", p["id"]);
        let default = p["region_default"].as_str().expect("region_default");
        assert!(
            regions.iter().any(|r| r.as_str() == Some(default)),
            "preset {}: region_default '{default}' not in its own region list",
            p["id"]
        );
    }
}
