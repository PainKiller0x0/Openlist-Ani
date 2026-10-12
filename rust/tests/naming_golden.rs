use serde::Deserialize;

use openlist_ani_rs::naming::{PlanError, build_plan};

#[derive(Deserialize)]
struct Fixture {
    #[serde(rename = "basePath")]
    base_path: String,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    input: serde_json::Value,
    expected: Expected,
}

#[derive(Deserialize)]
struct Expected {
    filename: Option<String>,
    directory: Option<String>,
    identity_season: Option<i32>,
    episode: Option<i32>,
    error: Option<String>,
    #[serde(rename = "submitCount")]
    submit_count: Option<u32>,
    #[serde(rename = "persistedConfigChanged")]
    persisted_config_changed: Option<bool>,
}

#[test]
fn download_plan_matches_independent_naming_golden_cases() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("../fixtures/golden-naming.json")).unwrap();

    assert_eq!(fixture.cases.len(), 14);
    for case in fixture.cases {
        let input = serde_json::from_value(case.input)
            .unwrap_or_else(|error| panic!("{} input: {error}", case.id));
        let actual = build_plan(&fixture.base_path, input);

        if let Some(expected_error) = case.expected.error {
            let error = actual.expect_err(&case.id);
            assert_eq!(error.code(), expected_error, "{}", case.id);
            if let Some(submit_count) = case.expected.submit_count {
                assert_eq!(submit_count, 0, "{}", case.id);
            }
            if let Some(changed) = case.expected.persisted_config_changed {
                assert!(!changed, "{}", case.id);
            }
            continue;
        }

        let plan = actual.unwrap_or_else(|error: PlanError| panic!("{}: {error}", case.id));
        assert_eq!(Some(plan.filename), case.expected.filename, "{}", case.id);
        assert_eq!(Some(plan.directory), case.expected.directory, "{}", case.id);
        if let Some(expected) = case.expected.identity_season {
            assert_eq!(plan.identity_season, expected, "{}", case.id);
        }
        if let Some(expected) = case.expected.episode {
            assert_eq!(plan.episode, expected, "{}", case.id);
        }
    }
}
