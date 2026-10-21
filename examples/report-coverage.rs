//! Summarize a private coverage campaign without merging compatibility and accuracy.
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn read(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn add(target: &mut BTreeMap<String, u64>, source: &Value) {
    for (key, value) in source.as_object().into_iter().flatten() {
        if let Some(value) = value.as_u64() {
            *target.entry(key.clone()).or_default() += value;
        }
    }
}

fn compatibility(reports: &[Value]) -> Value {
    let mut counts = BTreeMap::<String, BTreeMap<String, u64>>::new();
    let mut ratios = BTreeMap::<String, Vec<f64>>::new();
    for report in reports {
        let groups = std::iter::once(("broad", &report["fields"])).chain(
            report["exact_queries"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(_, fields)| ("exact", fields)),
        );
        for (kind, fields) in groups {
            for (field, item) in fields.as_object().into_iter().flatten() {
                for side in ["baseline", "product"] {
                    let key = format!("{kind}/{field}/{side}");
                    let target = counts.entry(key.clone()).or_default();
                    *target.entry("captured_project_queries".into()).or_default() += 1;
                    if report["run_finished"] != true
                        || report["comparison_eligible"] == false
                        || report["finalized"] != true
                        || report["launch"]["complete"] != true
                    {
                        *target.entry("ineligible_run_queries".into()).or_default() += 1;
                        continue;
                    }
                    if item["streams"]["reference"]["usable"] != true
                        || item["streams"][side]["usable"] != true
                    {
                        *target.entry("unusable_project_queries".into()).or_default() += 1;
                        continue;
                    }
                    let stats = &item["comparisons"][side];
                    if !stats.is_object() {
                        continue;
                    }
                    *target.entry("usable_project_queries".into()).or_default() += 1;
                    for key in ["matched_rows", "ripscope_only", "reference_only"] {
                        *target.entry(key.into()).or_default() += stats[key].as_u64().unwrap_or(0);
                    }
                    let matched = stats["matched_rows"].as_u64().unwrap_or(0);
                    let denominator = matched
                        + stats["ripscope_only"].as_u64().unwrap_or(0)
                        + stats["reference_only"].as_u64().unwrap_or(0);
                    if denominator > 0 {
                        ratios
                            .entry(key)
                            .or_default()
                            .push(matched as f64 / denominator as f64);
                    } else {
                        *target.entry("empty_pairs".into()).or_default() += 1;
                    }
                    if stats["ordered_equal"] == true {
                        *target.entry("ordered_equal_queries".into()).or_default() += 1;
                    }
                }
            }
        }
    }
    json!(counts.into_iter().map(|(key, values)| {
        let project_ratios = ratios.get(&key);
        let mean = project_ratios.filter(|items| !items.is_empty()).map(|items| items.iter().sum::<f64>() / items.len() as f64);
        (key, json!({"counts":values,"mean_project_query_multiset_overlap":mean,
                    "definition":"matched / (matched + reference_only + ripscope_only); empty pairs excluded from ratios"}))
    }).collect::<BTreeMap<_,_>>())
}

fn attribution(reports: &[Value]) -> Value {
    let mut counts = BTreeMap::<String, BTreeMap<String, u64>>::new();
    for report in reports {
        let groups = std::iter::once(("broad", &report["fields"])).chain(
            report["exact_queries"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(_, fields)| ("exact", fields)),
        );
        for (kind, fields) in groups {
            for (field, item) in fields.as_object().into_iter().flatten() {
                let category = if report["run_finished"] != true
                    || report["comparison_eligible"] == false
                    || report["finalized"] != true
                    || report["launch"]["complete"] != true
                {
                    "incomplete_measurement"
                } else if item["streams"]["reference"]["usable"] != true {
                    "reference_failure"
                } else if item["streams"]["product"]["usable"] != true {
                    "product_stream_failure"
                } else if let Some(reason) =
                    report["attribution_overrides"][format!("{kind}/{field}")].as_str()
                {
                    reason
                } else if item["comparisons"]["product"]["ordered_equal"] == true {
                    "agreement"
                } else if item["comparisons"]["product"]["reference_only"] == 0
                    && item["comparisons"]["product"]["ripscope_only"] == 0
                {
                    "query_ordering"
                } else if field == "1"
                    && item["streams"]["baseline"]["usable"] == true
                    && item["comparisons"]["baseline"]["ordered_equal"] == true
                {
                    "corrections"
                } else {
                    "unresolved"
                };
                *counts
                    .entry(format!("{kind}/{field}"))
                    .or_default()
                    .entry(category.into())
                    .or_default() += 1;
            }
        }
    }
    json!({"counts":counts,"criteria":{
        "corrections":"Definition rows differ only with Corrections enabled; the baseline agrees with the reference. This identifies the mechanism, not source correctness.",
        "query_ordering":"Row multisets agree; result ordering differs.",
        "reference_failure":"The pinned reference stream is unusable; excluded from agreement figures.",
        "unresolved":"These captures do not separate Corrections, query semantics and reference misreadings. Source-confirmed examples are recorded separately in findings."}})
}

fn stream_failures(report: &Value) -> Value {
    let groups = std::iter::once(("broad", ".*", &report["fields"])).chain(
        report["exact_queries"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(pattern, fields)| ("exact", pattern.as_str(), fields)),
    );
    json!(groups.flat_map(|(kind, pattern, fields)| {
        fields.as_object().into_iter().flatten().flat_map(move |(field, item)| {
            item["streams"].as_object().into_iter().flatten()
                .filter(|(_,stream)| stream["usable"] != true)
                .map(move |(side, stream)|json!({"kind":kind,"pattern":pattern,"field":field,"side":side,"stream":stream}))
        })
    }).collect::<Vec<_>>())
}

fn discovery_and_scanner(reports: &[Value]) -> Value {
    let eligible = reports
        .iter()
        .filter(|report| {
            report["run_finished"] == true
                && report["comparison_eligible"] != false
                && report["finalized"] == true
                && report["launch"]["complete"] == true
        })
        .collect::<Vec<_>>();
    let sum = |keys: &[&str]| {
        eligible
            .iter()
            .map(|report| {
                let mut value = *report;
                for key in keys {
                    value = &value[*key];
                }
                value.as_u64().unwrap_or(0)
            })
            .sum::<u64>()
    };
    json!({"eligible_projects":eligible.len(),"incomplete_projects":reports.len()-eligible.len(),
        "discovered_files":sum(&["files","reference"]),
        "matching_discovery_projects":eligible.iter().filter(|report|report["files"]["baseline_order_equal"]==true && report["files"]["product_order_equal"]==true).count(),
        "scanner_records":sum(&["scanner","records"]),"matching_scanner_records":sum(&["scanner","matching_records"]),
        "scanner_files":sum(&["scanner","files"]),"matching_scanner_files":sum(&["scanner","agreeing_files"]),
        "scanner_diagnostics_bytes":sum(&["scanner","diagnostics_bytes"])})
}

fn source(reports: &[Value]) -> Value {
    let mut totals = BTreeMap::<String, BTreeMap<String, u64>>::new();
    let mut constructs = BTreeMap::<String, BTreeMap<String, u64>>::new();
    let mut queries = BTreeMap::<String, BTreeMap<String, u64>>::new();
    let mut eligible = 0;
    for report in reports {
        if report["finalized"] != true
            || report["launch"]["complete"] != true
            || report["comparison_eligible"] == false
        {
            continue;
        }
        eligible += 1;
        for (field, value) in report["totals"].as_object().into_iter().flatten() {
            add(totals.entry(field.clone()).or_default(), value);
        }
        for (construct, value) in report["constructs"].as_object().into_iter().flatten() {
            add(constructs.entry(construct.clone()).or_default(), value);
        }
        for query in report["queries"].as_array().into_iter().flatten() {
            let counts = queries
                .entry(query["query"]["field"].to_string())
                .or_default();
            *counts
                .entry("captured_inventory_entries".into())
                .or_default() += 1;
            if query["unmeasured_reason"].is_string() {
                *counts.entry("unmeasured".into()).or_default() += 1;
            } else if query["query"]["unscored_reason"].is_string() {
                *counts.entry("unscored".into()).or_default() += 1;
            } else if query["query"]["field"] != 7
                && query["query"]["scopes"]
                    .as_array()
                    .is_some_and(|scopes| scopes.is_empty())
            {
                *counts.entry("unassessed_scope_queries".into()).or_default() += 1;
            } else if query["query"]["expected"]
                .as_array()
                .is_some_and(|rows| rows.is_empty())
            {
                *counts.entry("expected_empty".into()).or_default() += 1;
                if query["result"]["spurious"]
                    .as_array()
                    .is_some_and(|rows| rows.is_empty())
                {
                    *counts.entry("correct_empty".into()).or_default() += 1;
                }
            } else {
                *counts.entry("expected_positive".into()).or_default() += 1;
            }
        }
    }
    json!({"totals":totals,"constructs":constructs,"query_inventory":queries,
           "eligible_reports":eligible,"ineligible_reports":reports.len()-eligible})
}

fn summary(root: &Path, kind: &str, name: &str) -> Result<Value> {
    let path = root.join(format!("{kind}-summaries/{name}.json"));
    if path.exists() {
        let report = read(&path)?;
        let mut expected = read(&root.join("method-freeze.json"))?["identity"].clone();
        expected["hashes"]
            .as_object_mut()
            .ok_or("missing frozen hashes")?
            .retain(|file, _| {
                !file.contains('/')
                    || file.starts_with("examples/")
                    || file.starts_with("scripts/")
                    || file.starts_with("target/")
                    || file.starts_with("actual/")
                    || file == &format!("manifests/{name}.json")
                    || file == &format!("closures/{name}.json")
                    || file == &format!("exact/{name}.json")
                    || (kind == "deep" && file == &format!("sealed/{name}.json"))
            });
        let finished = report["run_finished"] == true || report["complete"] == true;
        if report["measurement_identity"] != expected
            || ((finished || report["binary_sha256"].is_string())
                && report["binary_sha256"] != expected["hashes"]["actual/binary"])
            || (kind == "deep"
                && (finished || report["manifest_sha256"].is_string())
                && report["manifest_sha256"] != expected["hashes"][format!("sealed/{name}.json")])
        {
            return Ok(
                json!({"complete":false,"status":"ineligible","error":"Summary identity differs from frozen method", "path":path}),
            );
        }
        Ok(report)
    } else {
        Ok(json!({"complete":false,"status":"unmeasured"}))
    }
}

fn run(root: &Path) -> Result<()> {
    let acquisition = read(&root.join("acquisition.json"))?;
    let draws = read(&root.join("draws.json"))?;
    let annotations = if root.join("outcome-annotations.json").exists() {
        read(&root.join("outcome-annotations.json"))?
    } else {
        json!({})
    };
    let mut projects = Vec::new();
    let mut broad = Vec::new();
    let mut deep = Vec::new();
    for project in acquisition["projects"]
        .as_array()
        .ok_or("missing project inventory")?
    {
        let name = project["name"].as_str().ok_or("project has no name")?;
        let mut comparison = summary(root, "broad", name)?;
        comparison["attribution_overrides"] = annotations[name]["attribution_overrides"].clone();
        broad.push(comparison.clone());
        let draw = draws["projects"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|draw| draw["name"] == name);
        let accuracy = if draw.is_some() {
            summary(root, "deep", name)?
        } else {
            Value::Null
        };
        if draw.is_some() {
            deep.push(accuracy.clone());
        }
        let closure = read(&root.join(format!("closures/{name}.json")))?;
        projects.push(json!({"name":name,"family":project["family"],"group":project["group"],"revision":project["revision"],"tag":project["tag"],
            "selection_files":project["files"].as_array().map(Vec::len),"exclusions":project["excluded"],"outcome_annotation":annotations[name],
            "test_example_files":project["files"].as_array().into_iter().flatten().filter(|file|file["test_or_example"]==true).count(),
            "generated_notice_files":project["files"].as_array().into_iter().flatten().filter(|file|file["generated_notice"]==true).count(),
            "missing_literal_operands":closure["missing_literal_operands"],"unsupported_inputs":closure["unsupported_inputs"],"closure_inventory_version":closure["inventory_version"],"acquisition_complete":project["complete"],
            "compatibility":{"complete":comparison["complete"],"run_finished":comparison["run_finished"],"comparison_eligible":comparison["comparison_eligible"],"unsupported_inputs":comparison["unsupported_inputs"],"input_policy":comparison["input_policy"],"status":comparison["status"],"error":comparison["error"],"files":comparison["files"],"scanner":comparison["scanner"],"figures":compatibility(std::slice::from_ref(&comparison)),"archive":comparison["archive"],"launch":comparison["launch"],
                "attribution":attribution(std::slice::from_ref(&comparison)),
                "indexing": (["baseline-build","product-build","reference-build"].into_iter().map(|name|(name,comparison["commands"][name].clone())).collect::<BTreeMap<_,_>>()),
                "stream_failures":stream_failures(&comparison),
                "diagnostic_commands":comparison["commands"].as_object().into_iter().flatten().filter(|(_,value)|value["stderr_bytes"].as_u64().unwrap_or(0)>0 || value["exit"]!=0 || !value["stopped"].is_null()).map(|(name,value)|json!({"name":name,"exit":value["exit"],"stderr_bytes":value["stderr_bytes"],"stopped":value["stopped"]})).collect::<Vec<_>>()},
            "draw":draw,"accuracy":if accuracy.is_null(){Value::Null}else{json!({"complete":accuracy["complete"],"error":accuracy["error"],"selected_files":accuracy["selected_files"],"assessed_files":accuracy["assessed_files"],"discovered_files":accuracy["discovered_files"],"figures":source(std::slice::from_ref(&accuracy)),"unresolved":accuracy["unresolved"],"archive":accuracy["archive"],"launch":accuracy["launch"]})}
        }));
    }
    let mut challenges = Vec::new();
    for name in ["memcached", "cjson", "libgit2", "micropython", "winfsp"] {
        let path = root.join(format!("challenges/{name}/report.json"));
        if path.exists() {
            let mut value = read(&path)?;
            let identities = read(&root.join("identities.json"))?;
            let manifest = root.join(format!("challenges/{name}/manifest.json"));
            use sha2::{Digest, Sha256};
            let manifest_sha =
                base16ct::lower::encode_string(&Sha256::digest(fs::read(&manifest)?));
            if value["binary_sha256"] != identities["binary_sha256"]
                || value["manifest_sha256"] != manifest_sha
            {
                value = json!({"complete":false,"status":"ineligible","error":"Challenge identity differs from frozen manifest/product", "path":path});
            }
            value["project"] = json!(name);
            let mut ranges = read(&manifest)?["queries"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|query| query["scopes"].as_array().into_iter().flatten().cloned())
                .collect::<Vec<_>>();
            ranges.sort_by_key(Value::to_string);
            ranges.dedup();
            value["assessed_ranges"] = json!(ranges);
            challenges.push(value);
        }
    }
    let report = json!({"scope":"Engineering sample; accuracy only in declared source scopes/query inventories. Call directions reuse the same source facts.","identities":read(&root.join("identities.json"))?,"method":read(&root.join("method-freeze.json"))?,
        "planned_projects":projects.len(),"planned_deep_projects":deep.len(),"planned_compatibility_pairs_per_reading":{"broad":projects.len()*7,"exact":projects.len()*9*7},"nonempty_source_draws":draws["projects"].as_array().into_iter().flatten().filter(|draw|draw["files"].as_array().is_some_and(|files|!files.is_empty())).count(),
        "broad_run_finished":broad.iter().filter(|value|value["run_finished"]==true).count(),"broad_complete":broad.iter().filter(|value|value["complete"]==true).count(),"deep_complete":deep.iter().filter(|value|value["complete"]==true).count(),
        "discovery_and_scanner":discovery_and_scanner(&broad),"compatibility":compatibility(&broad),"compatibility_attribution":attribution(&broad),"source_accuracy":source(&deep),"challenge_accuracy":source(&challenges),"challenge_inventory":read(&root.join("challenge-inventory.json"))?,"challenge_projects":challenges.iter().map(|report|json!({"name":report["project"],"complete":report["complete"],"assessed_ranges":report["assessed_ranges"],"figures":source(std::slice::from_ref(report))})).collect::<Vec<_>>(),"projects":projects,
        "windows":read(&root.join("windows.json"))?,"findings":if root.join("findings.json").exists(){read(&root.join("findings.json"))?}else{json!([])} });
    fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "{}",
        serde_json::to_string(
            &json!({"planned":report["planned_projects"],"broad_finished":report["broad_run_finished"],"broad_complete":report["broad_complete"],"deep_complete":report["deep_complete"],"nonempty_draws":report["nonempty_source_draws"]})
        )?
    );
    Ok(())
}

fn main() -> Result<()> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: report-coverage CAMPAIGN_ROOT")?;
    run(Path::new(&root))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unusable_reference_never_enters_aggregate_agreement() {
        let report = json!({"run_finished":true,"finalized":true,"launch":{"complete":true},"fields":{"1":{"streams":{"reference":{"usable":false},"product":{"usable":true}},"comparisons":{"product":{"matched_rows":99}}}}});
        let counts = compatibility(&[report]);
        assert_eq!(
            counts["broad/1/product"]["counts"]["unusable_project_queries"],
            1
        );
        assert!(counts["broad/1/product"]["counts"]["matched_rows"].is_null());
    }
    #[test]
    fn unmeasured_and_unscored_queries_are_not_empty_successes() {
        let report = json!({"finalized":true,"launch":{"complete":true},"queries":[{"query":{"field":1,"expected":[]},"unmeasured_reason":"timeout"},{"query":{"field":1,"expected":[],"unscored_reason":"ambiguity"},"result":{"spurious":[]}}]});
        let counts = source(&[report]);
        assert_eq!(counts["query_inventory"]["1"]["unmeasured"], 1);
        assert_eq!(counts["query_inventory"]["1"]["unscored"], 1);
        assert!(counts["query_inventory"]["1"]["correct_empty"].is_null());
    }
    #[test]
    fn unfinalized_runs_cannot_contribute_valid_looking_answers() {
        let report = json!({"run_finished":false,"finalized":false,"launch":{"complete":true},
          "fields":{"1":{"streams":{"reference":{"usable":true},"product":{"usable":true}},"comparisons":{"product":{"matched_rows":99}}}},
          "totals":{"1":{"correct":99}}});
        let counts = compatibility(std::slice::from_ref(&report));
        assert_eq!(
            counts["broad/1/product"]["counts"]["ineligible_run_queries"],
            1
        );
        assert!(counts["broad/1/product"]["counts"]["matched_rows"].is_null());
        assert_eq!(source(&[report])["ineligible_reports"], 1);
    }

    #[test]
    fn unsupported_inputs_exclude_even_complete_looking_streams() {
        let report = json!({"run_finished":true,"finalized":true,"launch":{"complete":true},
            "comparison_eligible":false,"status":"unsupported_inputs",
            "fields":{"1":{"streams":{"reference":{"usable":true},"product":{"usable":true}},
                "comparisons":{"product":{"matched_rows":99,"reference_only":0,"ripscope_only":0}}}},
            "totals":{"1":{"correct":99}},"scanner":{"matching_records":99}});
        let counts = compatibility(std::slice::from_ref(&report));
        assert_eq!(
            counts["broad/1/product"]["counts"]["ineligible_run_queries"],
            1
        );
        assert!(counts["broad/1/product"]["counts"]["matched_rows"].is_null());
        assert_eq!(
            attribution(std::slice::from_ref(&report))["counts"]["broad/1"]["incomplete_measurement"],
            1
        );
        assert_eq!(
            discovery_and_scanner(std::slice::from_ref(&report))["eligible_projects"],
            0
        );
        assert_eq!(source(&[report])["ineligible_reports"], 1);
    }

    #[test]
    fn a_summary_for_another_binary_or_method_is_ineligible() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("broad-summaries")).unwrap();
        let identity = json!({"version":1,"hashes":{"actual/binary":"frozen","manifests/demo.json":"selection"}});
        fs::write(
            directory.path().join("method-freeze.json"),
            json!({"identity":identity}).to_string(),
        )
        .unwrap();
        let path = directory.path().join("broad-summaries/demo.json");
        for report in [
            json!({"measurement_identity":identity,"binary_sha256":"changed"}),
            json!({"measurement_identity":{"version":2,"hashes":identity["hashes"]},"binary_sha256":"frozen"}),
        ] {
            fs::write(&path, report.to_string()).unwrap();
            assert_eq!(
                summary(directory.path(), "broad", "demo").unwrap()["status"],
                "ineligible"
            );
        }
    }

    #[test]
    fn an_empty_source_frame_is_not_a_correct_empty_answer() {
        let report = json!({"finalized":true,"launch":{"complete":true},"queries":[
            {"query":{"field":1,"expected":[],"scopes":[]},"result":{"spurious":[]}}
        ]});
        let counts = source(&[report]);
        assert_eq!(
            counts["query_inventory"]["1"]["unassessed_scope_queries"],
            1
        );
        assert!(counts["query_inventory"]["1"]["correct_empty"].is_null());
    }

    #[test]
    fn failed_partial_summary_preserves_its_failure_without_a_binary_result() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("broad-summaries")).unwrap();
        let identity = json!({"version":1,"hashes":{"actual/binary":"frozen"}});
        fs::write(
            directory.path().join("method-freeze.json"),
            json!({"identity":identity}).to_string(),
        )
        .unwrap();
        let report = json!({"measurement_identity":identity,"run_finished":false,"complete":false,
            "launch":{"complete":false},"error":"discovered unpinned input"});
        fs::write(
            directory.path().join("broad-summaries/demo.json"),
            report.to_string(),
        )
        .unwrap();
        assert_eq!(
            summary(directory.path(), "broad", "demo").unwrap()["error"],
            "discovered unpinned input"
        );
    }

    #[test]
    fn corrections_attribution_requires_a_matching_definition_baseline() {
        let item = json!({"streams":{"reference":{"usable":true},"baseline":{"usable":true},"product":{"usable":true}},
            "comparisons":{"baseline":{"ordered_equal":true},"product":{"ordered_equal":false,"reference_only":1,"ripscope_only":2}}});
        let report = json!({"run_finished":true,"finalized":true,"launch":{"complete":true},"fields":{"1":item,"3":item}});
        let counts = attribution(&[report]);
        assert_eq!(counts["counts"]["broad/1"]["corrections"], 1);
        assert_eq!(counts["counts"]["broad/3"]["unresolved"], 1);
    }
}
