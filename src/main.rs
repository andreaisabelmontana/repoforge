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
        println!("\n{} ({}/100, {})", snap.repo.full_name.bold(), a.score, a.grade);
        for action in &actions {
            planned += 1;
            if args.apply {
                match remediate::apply(gh, owner, name, action).await {
                    Ok(()) => {
                        applied += 1;
                        println!("  {} {}", "applied".green(), action.summary);
                    }
                    Err(e) => println!("  {} {}: {e}", "failed".red(), action.summary),
                }
            } else {
                println!("  {} {}", "would".cyan(), action.summary);
            }
        }
    }

    if args.apply {
        println!(
            "\n{} {applied}/{planned} change(s) applied across {touched} repo(s)",
            "done:".green().bold()
        );
    } else {
        println!(
            "\n{} {planned} change(s) across {touched} repo(s). Re-run with {} to apply.",
            "dry-run:".cyan().bold(),
            "--apply".bold()
        );
    }
    Ok(())
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
