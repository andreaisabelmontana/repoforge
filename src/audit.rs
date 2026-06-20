//! The scoring engine: a set of weighted checks run against a [`Snapshot`], producing a
//! 0–100 score, a letter grade, and a list of concrete, fixable gaps.

use crate::config::Config;
use crate::github::Snapshot;
use chrono::{DateTime, Utc};
use serde::Serialize;

/// A remediable gap. The variants the auto-fixer can act on are marked in `Remedy::fixable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Remedy {
    Description,
    Topics,
    Readme,
    ReadmeDepth,
    License,
    Gitignore,
    Ci,
    Tests,
    Homepage,
    Activity,
    Structure,
}

impl Remedy {
    /// Whether `repoforge fix` can generate and apply this automatically.
    pub fn fixable(self) -> bool {
        matches!(
            self,
            Remedy::Description
                | Remedy::Topics
                | Remedy::Readme
                | Remedy::License
                | Remedy::Gitignore
                | Remedy::Ci
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub id: &'static str,
    pub label: &'static str,
    pub weight: u32,
    pub passed: bool,
    pub detail: String,
    pub remedy: Remedy,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoAudit {
    pub full_name: String,
    pub language: Option<String>,
    pub score: u32,
    pub grade: char,
    pub checks: Vec<CheckResult>,
}

impl RepoAudit {
    /// Gaps that the auto-fixer can resolve, worst-weighted first.
    pub fn fixable_gaps(&self) -> Vec<&CheckResult> {
        let mut v: Vec<&CheckResult> = self
            .checks
            .iter()
            .filter(|c| !c.passed && c.remedy.fixable())
            .collect();
        v.sort_by_key(|c| std::cmp::Reverse(c.weight));
        v
    }

    pub fn failing(&self) -> impl Iterator<Item = &CheckResult> {
        self.checks.iter().filter(|c| !c.passed)
    }
}

pub fn grade_for(score: u32, cfg: &Config) -> char {
    let t = &cfg.thresholds;
    if score >= t.a {
        'A'
    } else if score >= t.b {
        'B'
    } else if score >= t.c {
        'C'
    } else if score >= t.d {
        'D'
    } else {
        'F'
    }
}

/// Run the full rubric against a snapshot.
pub fn audit(snap: &Snapshot, cfg: &Config) -> RepoAudit {
    audit_at(snap, cfg, Utc::now())
}

/// Same as [`audit`] but with an injectable "now" so the activity check is deterministic in tests.
pub fn audit_at(snap: &Snapshot, cfg: &Config, now: DateTime<Utc>) -> RepoAudit {
    let w = &cfg.weights;
    let lower: Vec<String> = snap.paths.iter().map(|p| p.to_lowercase()).collect();
    let r = &snap.repo;

    let mut checks = Vec::new();
    let mut push =
        |id: &'static str, label: &'static str, weight: u32, passed: bool, detail: String, remedy: Remedy| {
            checks.push(CheckResult {
                id,
                label,
                weight,
                passed,
                detail,
                remedy,
            });
        };

    // description
    let has_desc = r
        .description
        .as_deref()
        .map(str::trim)
        .is_some_and(|d| d.len() >= 10);
    push(
        "description",
        "Description",
        w.description,
        has_desc,
        if has_desc {
            "present".into()
        } else {
            "missing or too short".into()
        },
        Remedy::Description,
    );

    // topics
    let n_topics = r.topics.len();
    let has_topics = n_topics >= cfg.min_topics;
    push(
        "topics",
        "Topics",
        w.topics,
        has_topics,
        format!("{n_topics} topic(s), want >= {}", cfg.min_topics),
        Remedy::Topics,
    );

    // readme present
    let has_readme = snap.readme.is_some() || root_file_matches(&lower, "readme");
    push(
        "readme",
        "README present",
        w.readme,
        has_readme,
        if has_readme {
            "found".into()
        } else {
            "no README".into()
        },
        Remedy::Readme,
    );

    // readme depth
    let (chars, headings) = match &snap.readme {
        Some(t) => (
            t.chars().count(),
            t.lines().filter(|l| l.trim_start().starts_with('#')).count(),
        ),
        None => (0, 0),
    };
    let deep = chars >= cfg.readme_min_chars && headings >= 3;
    push(
        "readme_depth",
        "README depth",
        w.readme_depth,
        deep,
        format!(
            "{chars} chars, {headings} heading(s); want >= {} chars & 3 headings",
            cfg.readme_min_chars
        ),
        Remedy::ReadmeDepth,
    );

    // license
    let has_license = r
        .license
        .as_ref()
        .and_then(|l| l.spdx_id.as_deref())
        .is_some_and(|s| s != "NOASSERTION")
        || root_file_matches(&lower, "license")
        || root_file_matches(&lower, "licence")
        || root_file_matches(&lower, "copying");
    push(
        "license",
        "License",
        w.license,
        has_license,
        if has_license {
            "present".into()
        } else {
            "no license".into()
        },
        Remedy::License,
    );

    // .gitignore
    let has_gitignore = lower.iter().any(|p| p == ".gitignore");
    push(
        "gitignore",
        ".gitignore",
        w.gitignore,
        has_gitignore,
        if has_gitignore {
            "present".into()
        } else {
            "missing".into()
        },
        Remedy::Gitignore,
    );

    // CI
    let has_ci = lower
        .iter()
        .any(|p| p.starts_with(".github/workflows/") && (p.ends_with(".yml") || p.ends_with(".yaml")));
    push(
        "ci",
        "CI workflow",
        w.ci,
        has_ci,
        if has_ci {
            "GitHub Actions present".into()
        } else {
            "no workflow".into()
        },
        Remedy::Ci,
    );

    // tests
    let has_tests = detect_tests(&lower);
    push(
        "tests",
        "Tests",
        w.tests,
        has_tests,
        if has_tests {
            "test files detected".into()
        } else {
            "none detected".into()
        },
        Remedy::Tests,
    );

    // homepage / demo
    let has_home = r
        .homepage
        .as_deref()
        .map(str::trim)
        .is_some_and(|h| h.starts_with("http"));
    push(
        "homepage",
        "Homepage/demo",
        w.homepage,
        has_home,
        if has_home { "set".into() } else { "not set".into() },
        Remedy::Homepage,
    );

    // activity
    let (active, age) = match r.pushed_at.as_deref().and_then(parse_iso) {
        Some(ts) => {
            let days = (now - ts).num_days();
            (days <= 365, days)
        }
        None => (false, -1),
    };
    push(
        "activity",
        "Recent activity",
        w.activity,
        active,
        if age < 0 {
            "unknown".into()
        } else {
            format!("last push {age} day(s) ago")
        },
        Remedy::Activity,
    );

    // structure
    let structured = detect_structure(&lower);
    push(
        "structure",
        "Project structure",
        w.structure,
        structured,
        if structured {
            "organized layout".into()
        } else {
            "flat / unstructured".into()
        },
        Remedy::Structure,
    );

    let total: u32 = checks.iter().map(|c| c.weight).sum::<u32>().max(1);
    let earned: u32 = checks.iter().filter(|c| c.passed).map(|c| c.weight).sum();
    let score = (earned * 100 + total / 2) / total; // rounded
    RepoAudit {
        full_name: r.full_name.clone(),
        language: r.language.clone(),
        score,
        grade: grade_for(score, cfg),
        checks,
    }
}

fn root_file_matches(paths: &[String], stem: &str) -> bool {
    paths.iter().any(|p| !p.contains('/') && p.starts_with(stem))
}

fn detect_tests(paths: &[String]) -> bool {
    paths.iter().any(|p| {
        p.starts_with("test/")
            || p.starts_with("tests/")
            || p.contains("/test/")
            || p.contains("/tests/")
            || p.contains("__tests__/")
            || p.ends_with("_test.go")
            || p.ends_with("_test.py")
            || p.ends_with(".test.js")
            || p.ends_with(".test.ts")
            || p.ends_with(".spec.js")
            || p.ends_with(".spec.ts")
            || {
                let f = p.rsplit('/').next().unwrap_or(p);
                f.starts_with("test_") && f.ends_with(".py")
            }
    })
}

fn detect_structure(paths: &[String]) -> bool {
    const DIRS: [&str; 7] = ["src/", "lib/", "app/", "cmd/", "pkg/", "include/", "internal/"];
    if paths.iter().any(|p| DIRS.iter().any(|d| p.starts_with(d))) {
        return true;
    }
    // A static site counts as structured when it separates markup from assets.
    let has_index = paths.iter().any(|p| p == "index.html");
    let has_assets = paths
        .iter()
        .any(|p| p.ends_with(".css") || p.ends_with(".js") || p.starts_with("assets/"));
    has_index && has_assets
}

fn parse_iso(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}
