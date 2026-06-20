//! Integration tests for the pure logic: rubric scoring, grading, and every fix generator.
//! No network — everything runs against in-memory fixtures.

use chrono::{TimeZone, Utc};
use repoforge::audit::{self, grade_for, Remedy};
use repoforge::config::Config;
use repoforge::github::{License, Owner, Repo, Snapshot};
use repoforge::remediate::{self, ActionKind};
use repoforge::report;

fn repo() -> Repo {
    Repo {
        name: "gpu-montecarlo-risk".into(),
        full_name: "octocat/gpu-montecarlo-risk".into(),
        owner: Owner {
            login: "octocat".into(),
        },
        description: None,
        topics: vec![],
        language: Some("Rust".into()),
        license: None,
        default_branch: "main".into(),
        homepage: None,
        fork: false,
        archived: false,
        stargazers_count: 0,
        size: 0,
        pushed_at: None,
    }
}

fn deep_readme() -> String {
    let mut s = String::from("# Title\n\n## Usage\n\n## License\n\n");
    while s.len() < 900 {
        s.push_str("This repository does a real thing worth documenting in detail. ");
    }
    s
}

#[test]
fn grading_thresholds() {
    let c = Config::default();
    assert_eq!(grade_for(95, &c), 'A');
    assert_eq!(grade_for(85, &c), 'B');
    assert_eq!(grade_for(72, &c), 'C');
    assert_eq!(grade_for(61, &c), 'D');
    assert_eq!(grade_for(40, &c), 'F');
}

#[test]
fn empty_repo_fails() {
    let snap = Snapshot {
        repo: repo(),
        paths: vec![],
        readme: None,
        tree_truncated: false,
    };
    let now = Utc.with_ymd_and_hms(2024, 6, 1, 0, 0, 0).unwrap();
    let a = audit::audit_at(&snap, &Config::default(), now);
    assert_eq!(a.grade, 'F');
    assert!(a.score < 30, "bare repo should score low, got {}", a.score);
    assert!(a.fixable_gaps().iter().any(|g| g.id == "readme"));
}

#[test]
fn well_kept_repo_scores_a() {
    let mut r = repo();
    r.description = Some("GPU Monte-Carlo risk engine with a live training playground".into());
    r.topics = vec!["rust".into(), "gpu".into(), "simulation".into()];
    r.license = Some(License {
        spdx_id: Some("MIT".into()),
        name: Some("MIT License".into()),
    });
    r.homepage = Some("https://example.com".into());
    r.pushed_at = Some("2024-05-15T12:00:00Z".into());
    let snap = Snapshot {
        repo: r,
        paths: vec![
            "src/main.rs".into(),
            ".gitignore".into(),
            ".github/workflows/ci.yml".into(),
            "tests/sim.rs".into(),
            "README.md".into(),
        ],
        readme: Some(deep_readme()),
        tree_truncated: false,
    };
    let now = Utc.with_ymd_and_hms(2024, 6, 1, 0, 0, 0).unwrap();
    let a = audit::audit_at(&snap, &Config::default(), now);
    assert_eq!(a.grade, 'A', "score was {}", a.score);
    assert!(a.fixable_gaps().is_empty());
}

#[test]
fn gitignore_is_language_specific() {
    assert!(remediate::gen_gitignore(Some("Rust")).contains("/target"));
    assert!(remediate::gen_gitignore(Some("Python")).contains("__pycache__"));
    assert!(remediate::gen_gitignore(Some("Go")).contains("vendor/"));
    assert!(remediate::gen_gitignore(None).contains(".DS_Store"));
}

#[test]
fn ci_matches_language() {
    assert!(remediate::gen_ci(Some("Rust")).unwrap().contains("cargo test"));
    assert!(remediate::gen_ci(Some("Go")).unwrap().contains("go test"));
    assert!(remediate::gen_ci(Some("Java")).unwrap().contains("setup-java"));
    assert!(remediate::gen_ci(Some("C++")).unwrap().contains("cmake"));
    // Static sites get no default CI workflow.
    assert!(remediate::gen_ci(Some("HTML")).is_none());
    assert!(remediate::gen_ci(None).is_none());
}

#[test]
fn gitignore_covers_more_languages() {
    assert!(remediate::gen_gitignore(Some("C#")).contains("obj/"));
    assert!(remediate::gen_gitignore(Some("Ruby")).contains(".bundle/"));
    assert!(remediate::gen_gitignore(Some("PHP")).contains("/vendor/"));
}

#[test]
fn mit_license_carries_holder_and_year() {
    let text = remediate::gen_mit("Andrea Montana");
    assert!(text.starts_with("MIT License"));
    assert!(text.contains("Andrea Montana"));
}

#[test]
fn topics_derive_from_language_and_name() {
    let snap = Snapshot {
        repo: repo(),
        paths: vec!["Cargo.toml".into()],
        readme: None,
        tree_truncated: false,
    };
    let topics = remediate::suggest_topics(&snap);
    assert!(topics.contains(&"rust".into()));
    assert!(topics.contains(&"montecarlo".into()) || topics.contains(&"risk".into()));
    // Stopwords and short tokens must be filtered out.
    assert!(!topics.iter().any(|t| t == "gpu" && t.len() < 3));
}

#[test]
fn topics_fix_preserves_existing() {
    let mut r = repo();
    r.topics = vec!["hand-picked".into()]; // 1 topic -> fails the >=3 check, but must not be lost
    let snap = Snapshot {
        repo: r,
        paths: vec!["Cargo.toml".into()],
        readme: None,
        tree_truncated: false,
    };
    let now = Utc.with_ymd_and_hms(2024, 6, 1, 0, 0, 0).unwrap();
    let a = audit::audit_at(&snap, &Config::default(), now);
    let actions = remediate::plan(&snap, &a, &Some(vec![Remedy::Topics]), None);
    let topics_action = actions
        .iter()
        .find(|x| matches!(x.remedy, Remedy::Topics))
        .expect("a topics action");
    match &topics_action.kind {
        ActionKind::SetTopics(ts) => {
            assert!(
                ts.contains(&"hand-picked".to_string()),
                "existing topic must survive"
            );
            assert!(ts.len() > 1, "should add derived topics too");
        }
        _ => panic!("expected SetTopics"),
    }
}

#[test]
fn badge_reflects_grade() {
    let snap = Snapshot {
        repo: repo(),
        paths: vec![],
        readme: None,
        tree_truncated: false,
    };
    let now = Utc.with_ymd_and_hms(2024, 6, 1, 0, 0, 0).unwrap();
    let a = audit::audit_at(&snap, &Config::default(), now); // bare repo -> F
    let md = report::badge_markdown(&a);
    assert!(md.contains("img.shields.io/badge/repoforge-"));
    assert!(md.contains("-red)")); // F is red
    let json = report::badge_endpoint(&a);
    assert!(json.contains("\"schemaVersion\":1"));
    assert!(json.contains("\"color\":\"red\""));
}

#[test]
fn html_report_is_self_contained() {
    let snap = Snapshot {
        repo: repo(),
        paths: vec![],
        readme: None,
        tree_truncated: false,
    };
    let now = Utc.with_ymd_and_hms(2024, 6, 1, 0, 0, 0).unwrap();
    let a = audit::audit_at(&snap, &Config::default(), now);
    let html = report::html(std::slice::from_ref(&a));
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains("octocat/gpu-montecarlo-risk"));
    assert!(html.contains("Repository quality report"));
    // No external assets — everything inlined.
    assert!(!html.contains("http-equiv=\"refresh\""));
    assert!(html.contains("<style>"));
}

#[test]
fn readme_is_fact_grounded() {
    let mut r = repo();
    r.description = Some("Real description here".into());
    let snap = Snapshot {
        repo: r,
        paths: vec!["src/main.rs".into(), "Cargo.toml".into()],
        readme: None,
        tree_truncated: false,
    };
    let md = remediate::gen_readme(&snap);
    assert!(md.contains("# Gpu Montecarlo Risk"));
    assert!(md.contains("Real description here"));
    assert!(md.contains("Rust"));
    assert!(md.contains("TODO")); // honest placeholders, not invented prose
}
