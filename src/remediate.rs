//! Fix generation. Each generator produces *fact-grounded* content from what the snapshot
//! actually contains — no invented prose. Unknowns are left as explicit `TODO` markers so a
//! human fills them in, rather than the tool fabricating a description.

use crate::audit::{Remedy, RepoAudit};
use crate::github::{GitHub, Snapshot};
use anyhow::Result;
use chrono::{Datelike, Utc};

/// A single concrete change the fixer will make.
pub struct Action {
    pub remedy: Remedy,
    pub kind: ActionKind,
    /// Human summary shown in dry-run mode.
    pub summary: String,
}

pub enum ActionKind {
    /// Create a file at `path` with `contents`.
    PutFile { path: String, contents: String },
    /// PATCH the repo `description`.
    SetDescription(String),
    /// Replace the repo topic list.
    SetTopics(Vec<String>),
}

/// Build the list of actions that would resolve a repo's fixable gaps, optionally filtered to
/// a subset of remedies (`only`). `holder` overrides the license copyright holder (defaults to
/// the repo owner's login).
pub fn plan(
    snap: &Snapshot,
    audit: &RepoAudit,
    only: &Option<Vec<Remedy>>,
    holder: Option<&str>,
) -> Vec<Action> {
    let mut actions = Vec::new();
    for gap in audit.fixable_gaps() {
        if let Some(filter) = only {
            if !filter.contains(&gap.remedy) {
                continue;
            }
        }
        match gap.remedy {
            Remedy::Readme => actions.push(Action {
                remedy: Remedy::Readme,
                kind: ActionKind::PutFile {
                    path: "README.md".into(),
                    contents: gen_readme(snap),
                },
                summary: "create README.md".into(),
            }),
            Remedy::License => actions.push(Action {
                remedy: Remedy::License,
                kind: ActionKind::PutFile {
                    path: "LICENSE".into(),
                    contents: gen_mit(holder.unwrap_or(&snap.repo.owner.login)),
                },
                summary: "add MIT LICENSE".into(),
            }),
            Remedy::Gitignore => actions.push(Action {
                remedy: Remedy::Gitignore,
                kind: ActionKind::PutFile {
                    path: ".gitignore".into(),
                    contents: gen_gitignore(snap.repo.language.as_deref()),
                },
                summary: format!(
                    ".gitignore for {}",
                    snap.repo.language.as_deref().unwrap_or("generic")
                ),
            }),
            Remedy::Ci => {
                if let Some(ci) = gen_ci(snap.repo.language.as_deref()) {
                    actions.push(Action {
                        remedy: Remedy::Ci,
                        kind: ActionKind::PutFile {
                            path: ".github/workflows/ci.yml".into(),
                            contents: ci,
                        },
                        summary: "add CI workflow".into(),
                    });
                }
            }
            Remedy::Description => {
                // Only safe to auto-set when we can derive something real (language + name).
                if let Some(desc) = gen_description(snap) {
                    actions.push(Action {
                        remedy: Remedy::Description,
                        kind: ActionKind::SetDescription(desc),
                        summary: "set repo description".into(),
                    });
                }
            }
            Remedy::Topics => {
                let topics = suggest_topics(snap);
                if !topics.is_empty() {
                    actions.push(Action {
                        remedy: Remedy::Topics,
                        kind: ActionKind::SetTopics(topics.clone()),
                        summary: format!("set topics: {}", topics.join(", ")),
                    });
                }
            }
            _ => {}
        }
    }
    actions
}

/// Apply a planned action against GitHub. Caller decides whether to call this (dry-run vs apply).
/// When `branch` is `Some`, file changes land on that branch (used by `--pr` mode); metadata
/// changes (description/topics) always apply to the repo directly.
pub async fn apply(
    gh: &GitHub,
    owner: &str,
    name: &str,
    action: &Action,
    branch: Option<&str>,
) -> Result<()> {
    match &action.kind {
        ActionKind::PutFile { path, contents } => {
            gh.put_file(
                owner,
                name,
                path,
                &format!("chore: {}", action.summary),
                contents,
                branch,
            )
            .await
        }
        ActionKind::SetDescription(desc) => {
            gh.patch_repo(owner, name, serde_json::json!({ "description": desc }))
                .await
        }
        ActionKind::SetTopics(topics) => gh.replace_topics(owner, name, topics).await,
    }
}

impl ActionKind {
    /// Whether this change touches files (vs repo metadata). Only file changes can go in a PR.
    pub fn is_file(&self) -> bool {
        matches!(self, ActionKind::PutFile { .. })
    }
}

// ---------------------------------------------------------------------------
// Generators (pure functions — unit-tested without any network)
// ---------------------------------------------------------------------------

pub fn gen_description(snap: &Snapshot) -> Option<String> {
    let lang = snap.repo.language.as_deref()?;
    let pretty = title_case(&snap.repo.name);
    Some(format!("{pretty} — a {lang} project. TODO: one-line summary."))
}

pub fn suggest_topics(snap: &Snapshot) -> Vec<String> {
    let mut topics: Vec<String> = Vec::new();
    let mut push = |t: String| {
        let t = sanitize_topic(&t);
        if !t.is_empty() && t.len() <= 50 && !topics.contains(&t) {
            topics.push(t);
        }
    };

    if let Some(lang) = &snap.repo.language {
        push(lang.clone());
    }
    for tok in snap.repo.name.split(['-', '_', ' ']) {
        if tok.len() >= 3 && !STOPWORDS.contains(&tok.to_lowercase().as_str()) {
            push(tok.to_string());
        }
    }
    // Coarse archetype tags from the file tree (case-insensitive: GitHub preserves path case).
    let lower: Vec<String> = snap.paths.iter().map(|p| p.to_lowercase()).collect();
    if lower.iter().any(|p| p == "index.html") {
        push("website".into());
    }
    if lower.iter().any(|p| p.contains("dockerfile")) {
        push("docker".into());
    }
    if lower.iter().any(|p| p == "cargo.toml") {
        push("rust".into());
    }

    topics.truncate(10);
    topics
}

pub fn gen_mit(holder: &str) -> String {
    let year = Utc::now().year();
    format!(
        "MIT License\n\n\
Copyright (c) {year} {holder}\n\n\
Permission is hereby granted, free of charge, to any person obtaining a copy\n\
of this software and associated documentation files (the \"Software\"), to deal\n\
in the Software without restriction, including without limitation the rights\n\
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell\n\
copies of the Software, and to permit persons to whom the Software is\n\
furnished to do so, subject to the following conditions:\n\n\
The above copyright notice and this permission notice shall be included in all\n\
copies or substantial portions of the Software.\n\n\
THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR\n\
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,\n\
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE\n\
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER\n\
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,\n\
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE\n\
SOFTWARE.\n"
    )
}

pub fn gen_gitignore(language: Option<&str>) -> String {
    match language.map(str::to_lowercase).as_deref() {
        Some("rust") => "/target\n**/*.rs.bk\nCargo.lock\n".into(),
        Some("python") => {
            "__pycache__/\n*.py[cod]\n.venv/\nvenv/\n.env\n*.egg-info/\ndist/\nbuild/\n.pytest_cache/\n.mypy_cache/\n".into()
        }
        Some("javascript") | Some("typescript") => {
            "node_modules/\ndist/\nbuild/\n.env\n.env.local\nnpm-debug.log*\ncoverage/\n.next/\n".into()
        }
        Some("go") => "/bin/\n*.exe\n*.test\n*.out\nvendor/\n".into(),
        Some("java") | Some("kotlin") => "target/\n*.class\n*.jar\n.gradle/\nbuild/\n".into(),
        Some("c") | Some("c++") => "*.o\n*.obj\n*.exe\n*.out\nbuild/\n*.a\n*.so\n".into(),
        Some("c#") => "bin/\nobj/\n*.user\n.vs/\n*.suo\n".into(),
        Some("ruby") => "*.gem\n.bundle/\nvendor/bundle\nlog/\ntmp/\n.env\n".into(),
        Some("php") => "/vendor/\ncomposer.lock\n.env\n.phpunit.result.cache\n".into(),
        Some("swift") => ".build/\n*.xcodeproj/\nDerivedData/\n.swiftpm/\n".into(),
        _ => ".DS_Store\nThumbs.db\n*.log\n.env\nnode_modules/\ndist/\nbuild/\n".into(),
    }
}

/// A CI workflow appropriate to the detected language. `None` when no useful default exists.
pub fn gen_ci(language: Option<&str>) -> Option<String> {
    let body = match language.map(str::to_lowercase).as_deref() {
        Some("rust") => {
            "      - uses: actions/checkout@v4\n\
\x20     - uses: dtolnay/rust-toolchain@stable\n\
\x20       with: { components: rustfmt, clippy }\n\
\x20     - run: cargo fmt --all -- --check\n\
\x20     - run: cargo clippy --all-targets -- -D warnings\n\
\x20     - run: cargo test --all\n"
        }
        Some("python") => {
            "      - uses: actions/checkout@v4\n\
\x20     - uses: actions/setup-python@v5\n\
\x20       with: { python-version: '3.12' }\n\
\x20     - run: pip install -e . || pip install -r requirements.txt || true\n\
\x20     - run: pip install pytest && pytest -q || true\n"
        }
        Some("javascript") | Some("typescript") => {
            "      - uses: actions/checkout@v4\n\
\x20     - uses: actions/setup-node@v4\n\
\x20       with: { node-version: '20' }\n\
\x20     - run: npm ci || npm install\n\
\x20     - run: npm test --if-present\n"
        }
        Some("go") => {
            "      - uses: actions/checkout@v4\n\
\x20     - uses: actions/setup-go@v5\n\
\x20       with: { go-version: 'stable' }\n\
\x20     - run: go build ./...\n\
\x20     - run: go test ./...\n"
        }
        Some("java") => {
            "      - uses: actions/checkout@v4\n\
\x20     - uses: actions/setup-java@v4\n\
\x20       with: { distribution: temurin, java-version: '21' }\n\
\x20     - run: mvn -B verify || ./gradlew build || echo 'no build tool detected'\n"
        }
        Some("c") | Some("c++") => {
            "      - uses: actions/checkout@v4\n\
\x20     - run: cmake -B build && cmake --build build || make || echo 'no build system detected'\n\
\x20     - run: ctest --test-dir build || true\n"
        }
        // HTML / static sites and unknowns get nothing here; Pages deploy is a separate concern.
        _ => return None,
    };
    Some(format!(
        "name: CI\n\non:\n  push:\n    branches: [ main, master ]\n  pull_request:\n\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n{body}"
    ))
}

/// A README scaffold built only from facts we can observe. Sections we can't fill are marked TODO.
pub fn gen_readme(snap: &Snapshot) -> String {
    let r = &snap.repo;
    let title = title_case(&r.name);
    let desc = r
        .description
        .as_deref()
        .filter(|d| !d.trim().is_empty())
        .unwrap_or("<!-- TODO: one-sentence description -->");

    let mut s = String::new();
    s.push_str(&format!("# {title}\n\n{desc}\n\n"));

    if let Some(lang) = &r.language {
        s.push_str(&format!("**Stack:** {lang}\n\n"));
    }

    // Top-level layout, derived from the real tree.
    let top = top_level(&snap.paths);
    if !top.is_empty() {
        s.push_str("## Structure\n\n```\n");
        for entry in &top {
            s.push_str(entry);
            s.push('\n');
        }
        s.push_str("```\n\n");
    }

    // Getting-started commands keyed off the language.
    if let Some(cmds) = getting_started(r.language.as_deref()) {
        s.push_str("## Getting started\n\n```bash\n");
        s.push_str(cmds);
        s.push_str("```\n\n");
    }

    s.push_str("## Overview\n\n<!-- TODO: what this does, why it exists, how it works -->\n\n");
    s.push_str("## License\n\nMIT.\n");
    s
}

fn getting_started(language: Option<&str>) -> Option<&'static str> {
    match language.map(str::to_lowercase).as_deref() {
        Some("rust") => Some("cargo build --release\ncargo run\ncargo test\n"),
        Some("python") => Some("python -m venv .venv && source .venv/bin/activate\npip install -r requirements.txt\npython main.py\n"),
        Some("javascript") | Some("typescript") => Some("npm install\nnpm start\nnpm test\n"),
        Some("go") => Some("go build ./...\ngo run .\ngo test ./...\n"),
        _ => None,
    }
}

fn top_level(paths: &[String]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for p in paths {
        let head = match p.split_once('/') {
            Some((dir, _)) => format!("{dir}/"),
            None => p.clone(),
        };
        if !seen.contains(&head) {
            seen.push(head);
        }
    }
    seen.sort();
    seen.truncate(15);
    seen
}

fn sanitize_topic(t: &str) -> String {
    let s: String = t
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    s.trim_matches('-').to_string()
}

fn title_case(name: &str) -> String {
    name.split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut ch = w.chars();
            match ch.next() {
                Some(f) => f.to_uppercase().collect::<String>() + ch.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

const STOPWORDS: [&str; 10] = [
    "the", "and", "for", "with", "app", "rebuild", "project", "demo", "main", "new",
];
