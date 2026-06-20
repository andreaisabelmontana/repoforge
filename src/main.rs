//! repoforge — audit GitHub repositories against a quality rubric and auto-generate the
//! missing pieces. See `README.md` for the why; this file is the orchestration glue.

mod cli;

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use cli::{AuditArgs, Cli, Command, FixArgs, Format};
use colored::Colorize;
use futures::stream::{self, StreamExt};
use repoforge::audit;
use repoforge::config::Config;
use repoforge::github::{GitHub, Repo, Snapshot};
use repoforge::remediate;
use repoforge::report;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("{} {e:#}", "error:".red().bold());
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let cfg = Config::load_or_default(cli.config.as_deref())?;
    let token = resolve_token(cli.token.clone());
    if token.is_none() {
        eprintln!(
            "{} no token found; running anonymously (60 req/hour, public data only)",
            "warning:".yellow().bold()
        );
    }
    let gh = GitHub::new(token)?;

    match cli.command {
        Command::Audit(args) => audit_cmd(&gh, &cfg, cli.concurrency, args).await,
        Command::Fix(args) => fix_cmd(&gh, &cfg, cli.concurrency, args).await,
    }
}

/// Collect the target repositories from explicit `owner/name` args and/or a `--user` sweep.
async fn collect_repos(
    gh: &GitHub,
    explicit: &[String],
    user: &Option<String>,
    include_forks: bool,
    include_archived: bool,
) -> Result<Vec<Repo>> {
    let mut repos = Vec::new();
    if let Some(u) = user {
        eprintln!("Listing repositories for {}…", u.cyan());
        repos.extend(gh.list_user_repos(u, include_forks, include_archived).await?);
    }
    for spec in explicit {
        let (owner, name) = spec
            .split_once('/')
            .ok_or_else(|| anyhow!("repo must be in owner/name form: {spec}"))?;
        repos.push(gh.get_repo(owner, name).await?);
    }
    if repos.is_empty() {
        return Err(anyhow!(
            "no repositories selected — pass owner/name or --user <login>"
        ));
    }
    Ok(repos)
}

/// Fetch snapshots for many repos with bounded concurrency, reporting progress to stderr.
async fn snapshot_all(gh: &GitHub, repos: Vec<Repo>, concurrency: usize) -> Vec<Snapshot> {
    let total = repos.len();
    let done = std::sync::atomic::AtomicUsize::new(0);
    stream::iter(repos)
        .map(|repo| {
            let gh = &gh;
            let done = &done;
            async move {
                let full = repo.full_name.clone();
                let snap = gh.snapshot(repo).await;
                let n = done.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                match snap {
                    Ok(s) => {
                        eprintln!("  [{n}/{total}] {full}");
                        Some(s)
                    }
                    Err(e) => {
                        eprintln!("  [{n}/{total}] {} {full}: {e}", "skip".yellow());
                        None
                    }
                }
            }
        })
        .buffer_unordered(concurrency.max(1))
        .filter_map(|x| async move { x })
        .collect()
        .await
}

async fn audit_cmd(gh: &GitHub, cfg: &Config, concurrency: usize, args: AuditArgs) -> Result<()> {
    let repos = collect_repos(
        gh,
        &args.repos,
        &args.user,
        args.include_forks,
        args.include_archived,
    )
    .await?;
    let snaps = snapshot_all(gh, repos, concurrency).await;
    let audits: Vec<_> = snaps.iter().map(|s| audit::audit(s, cfg)).collect();

    let rendered = match args.format {
        Format::Table => format!("{}\n{}", report::table(&audits), report::summary(&audits)),
        Format::Summary => report::summary(&audits),
        Format::Json => report::json(&audits),
        Format::Markdown => report::markdown(&audits),
        Format::Html => report::html(&audits),
    };

    if let Some(path) = args.output {
        std::fs::write(&path, rendered).with_context(|| format!("writing {}", path.display()))?;
        eprintln!("{} wrote report to {}", "ok:".green().bold(), path.display());
    } else {
        println!("{rendered}");
    }
    Ok(())
}

async fn fix_cmd(gh: &GitHub, cfg: &Config, concurrency: usize, args: FixArgs) -> Result<()> {
    let only: Option<Vec<audit::Remedy>> = args
        .only
        .as_ref()
        .map(|v| v.iter().copied().map(Into::into).collect());

    let repos = collect_repos(
        gh,
        &args.repos,
        &args.user,
        args.include_forks,
        args.include_archived,
    )
    .await?;
    let snaps = snapshot_all(gh, repos, concurrency).await;

    let mut planned = 0usize;
    let mut applied = 0usize;
    let mut touched = 0usize;

    for snap in &snaps {
        let a = audit::audit(snap, cfg);
        if a.score > args.max_score {
            continue;
        }
        let actions = remediate::plan(snap, &a, &only);
        if actions.is_empty() {
            continue;
        }
        touched += 1;
        let (owner, name) = (snap.repo.owner.login.as_str(), snap.repo.name.as_str());
        let base = snap.repo.default_branch.as_str();
        println!("\n{} ({}/100, {})", snap.repo.full_name.bold(), a.score, a.grade);

        // In --pr mode, set up a working branch for the file changes up front.
        let has_file_changes = actions.iter().any(|x| x.kind.is_file());
        let branch = if args.pr && has_file_changes {
            match setup_branch(gh, owner, name, base).await {
                Ok(b) => Some(b),
                Err(e) => {
                    println!("  {} prepare branch: {e}", "failed".red());
                    None
                }
            }
        } else {
            None
        };

        for action in &actions {
            planned += 1;
            if !args.apply && !args.pr {
                println!("  {} {}", "would".cyan(), action.summary);
                continue;
            }
            // File changes go to the PR branch (if any); metadata always applies directly.
            let target = if action.kind.is_file() {
                branch.as_deref()
            } else {
                None
            };
            match remediate::apply(gh, owner, name, action, target).await {
                Ok(()) => {
                    applied += 1;
                    println!("  {} {}", "applied".green(), action.summary);
                }
                Err(e) => println!("  {} {}: {e}", "failed".red(), action.summary),
            }
        }

        // Open the PR once the branch carries the file changes.
        if let Some(b) = &branch {
            let body = pr_body(&actions);
            match gh
                .open_pr(owner, name, b, base, "chore: repoforge quality fixes", &body)
                .await
            {
                Ok(url) => println!("  {} {url}", "PR →".green().bold()),
                Err(e) => println!("  {} open PR: {e}", "failed".red()),
            }
        }
    }

    let mode = if args.pr {
        "via pull request"
    } else if args.apply {
        "applied directly"
    } else {
        "dry-run"
    };
    if args.apply || args.pr {
        println!(
            "\n{} {applied}/{planned} change(s) {mode} across {touched} repo(s)",
            "done:".green().bold()
        );
    } else {
        println!(
            "\n{} {planned} change(s) across {touched} repo(s). Re-run with {} or {} to apply.",
            "dry-run:".cyan().bold(),
            "--apply".bold(),
            "--pr".bold()
        );
    }
    Ok(())
}

/// Create (or reuse) the `repoforge/quality-fixes` branch off `base`, returning its name.
async fn setup_branch(gh: &GitHub, owner: &str, name: &str, base: &str) -> Result<String> {
    let sha = gh.head_sha(owner, name, base).await?;
    let branch = "repoforge/quality-fixes";
    gh.create_branch(owner, name, branch, &sha).await?;
    Ok(branch.to_string())
}

/// Markdown body listing the file changes a PR introduces.
fn pr_body(actions: &[remediate::Action]) -> String {
    let mut s = String::from(
        "Automated, additive quality fixes. Each file is generated from facts already in the repo.\n\n",
    );
    for a in actions.iter().filter(|x| x.kind.is_file()) {
        s.push_str(&format!("- {}\n", a.summary));
    }
    s
}

/// Token precedence: explicit flag → $GITHUB_TOKEN → $GH_TOKEN → `gh auth token`.
fn resolve_token(explicit: Option<String>) -> Option<String> {
    if let Some(t) = explicit.filter(|t| !t.trim().is_empty()) {
        return Some(t);
    }
    for var in ["GITHUB_TOKEN", "GH_TOKEN"] {
        if let Ok(t) = std::env::var(var) {
            if !t.trim().is_empty() {
                return Some(t);
            }
        }
    }
    let out = std::process::Command::new("gh")
        .args(["auth", "token"])
        .output()
        .ok()?;
    if out.status.success() {
        let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !t.is_empty() {
            return Some(t);
        }
    }
    None
}
