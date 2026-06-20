//! Command-line surface, defined with `clap`'s derive API.

use clap::{Args, Parser, Subcommand, ValueEnum};
use repoforge::audit::Remedy;

#[derive(Parser, Debug)]
#[command(
    name = "repoforge",
    version,
    about = "Audit GitHub repositories against a quality rubric and auto-generate what's missing.",
    long_about = None
)]
pub struct Cli {
    /// GitHub token. Falls back to $GITHUB_TOKEN, then $GH_TOKEN, then `gh auth token`.
    #[arg(long, global = true)]
    pub token: Option<String>,

    /// Max repositories fetched/audited concurrently.
    #[arg(long, global = true, default_value_t = 8)]
    pub concurrency: usize,

    /// Path to a repoforge.toml config (defaults to ./repoforge.toml if present).
    #[arg(long, global = true)]
    pub config: Option<std::path::PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Score one or more repositories (or every repo a user owns).
    Audit(AuditArgs),
    /// Generate and (optionally) apply the missing pieces for failing checks.
    Fix(FixArgs),
}

#[derive(Args, Debug)]
pub struct AuditArgs {
    /// Explicit repositories as `owner/name`. Mutually combinable with --user.
    pub repos: Vec<String>,

    /// Audit every repository owned by this login.
    #[arg(long)]
    pub user: Option<String>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Table)]
    pub format: Format,

    /// Write output to a file instead of stdout.
    #[arg(long)]
    pub output: Option<std::path::PathBuf>,

    /// Include forked repositories.
    #[arg(long)]
    pub include_forks: bool,

    /// Include archived repositories.
    #[arg(long)]
    pub include_archived: bool,
}

#[derive(Args, Debug)]
pub struct FixArgs {
    /// Explicit repositories as `owner/name`.
    pub repos: Vec<String>,

    /// Fix every repository owned by this login.
    #[arg(long)]
    pub user: Option<String>,

    /// Actually push changes. Without this (and without --pr), runs as a dry-run.
    #[arg(long)]
    pub apply: bool,

    /// Apply file fixes via a reviewable pull request (branch `repoforge/quality-fixes`)
    /// instead of committing to the default branch.
    #[arg(long)]
    pub pr: bool,

    /// Restrict to specific remedies, comma-separated:
    /// description,topics,readme,license,gitignore,ci
    #[arg(long, value_delimiter = ',')]
    pub only: Option<Vec<RemedyArg>>,

    /// Only touch repositories scoring at or below this value.
    #[arg(long, default_value_t = 100)]
    pub max_score: u32,

    /// Copyright holder for generated LICENSE files (defaults to the repo owner's login).
    #[arg(long)]
    pub holder: Option<String>,

    #[arg(long)]
    pub include_forks: bool,

    #[arg(long)]
    pub include_archived: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Table,
    Summary,
    Json,
    Markdown,
    Html,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum RemedyArg {
    Description,
    Topics,
    Readme,
    License,
    Gitignore,
    Ci,
}

impl From<RemedyArg> for Remedy {
    fn from(r: RemedyArg) -> Self {
        match r {
            RemedyArg::Description => Remedy::Description,
            RemedyArg::Topics => Remedy::Topics,
            RemedyArg::Readme => Remedy::Readme,
            RemedyArg::License => Remedy::License,
            RemedyArg::Gitignore => Remedy::Gitignore,
            RemedyArg::Ci => Remedy::Ci,
        }
    }
}
