# repoforge

[![CI](https://github.com/andreaisabelmontana/repoforge/actions/workflows/ci.yml/badge.svg)](https://github.com/andreaisabelmontana/repoforge/actions/workflows/ci.yml)
![license: MIT](https://img.shields.io/badge/license-MIT-blue)
![repoforge quality](https://img.shields.io/badge/repoforge-A%20%28100%29-brightgreen)

**[Live quality dashboard →](https://andreaisabelmontana.github.io/repoforge/)** (auto-refreshed weekly by the self-audit workflow)

Audit GitHub repositories against a quality rubric and auto-generate the pieces they're missing — README, license, `.gitignore`, CI workflow, description, and topics. One read-only command grades an entire account; one `--apply` flag fixes it.

Built because a profile with 200+ repos rots one missing README at a time, and fixing that by hand doesn't scale. repoforge makes "every repo meets a bar" a command instead of a chore.

> Proven at scale on a 200+ repo account. Safe, additive `fix --apply` runs added an MIT license + language-appropriate `.gitignore` to **162 repositories** and derived topics to **135** — hundreds of changes in two passes, zero failures. Net effect on the account: average score **51.8 → 69.6**, A-grade repos **4 → 19**, and *license*, *.gitignore* and *topics* all driven off the most-common-gaps list. The kind of fleet-wide cleanup that is otherwise weeks of copy-paste.

## What it does

- **Scores** any repo (or every repo a user owns) 0–100 across 11 weighted checks, producing a letter grade and a ranked list of gaps.
- **Generates** the missing pieces from facts it can actually observe — never invented prose. Unknowns become explicit `TODO` markers, not hallucinated descriptions.
- **Applies** fixes through the GitHub API behind an explicit `--apply` flag (direct commits) or `--pr` (reviewable pull request). Default is always a dry-run.
- **Runs concurrently** — bounded parallel fetches, so auditing hundreds of repos takes seconds, not minutes.
- **Reports five ways** — colored terminal table, summary rollup, committable markdown, machine-readable JSON, or a self-contained HTML dashboard.
- **Survives rate limits** — reads *and* writes retry with exponential backoff, honouring the `Retry-After` header (including GitHub's secondary-limit 403 on rapid writes).

## Install

```bash
cargo install --path .
# or, from a clone:
cargo build --release   # binary at target/release/repoforge
```

A GitHub token is read from `--token`, then `$GITHUB_TOKEN`, then `$GH_TOKEN`, then `gh auth token`. Without one it runs anonymously (public data, 60 req/hour).

## Usage

```bash
# Grade every repo an account owns, worst-first, with a summary rollup
repoforge audit --user octocat

# Grade specific repos, emit a committable markdown report
repoforge audit octocat/hello octocat/spoon-knife --format markdown --output report.md

# Self-contained HTML dashboard (open in a browser or publish to Pages)
repoforge audit --user octocat --format html --output dashboard.html

# Machine-readable
repoforge audit --user octocat --format json

# Use as a CI quality gate — non-zero exit if any repo scores below 70
repoforge audit octocat/hello --fail-under 70

# A ready-to-paste shields.io quality badge for a repo's README
repoforge badge octocat/hello
repoforge badge octocat/hello --json   # endpoint schema, for a self-updating badge

# Create a brand-new repo already scaffolded to an A grade
repoforge init my-new-tool --language rust --description "A small fast thing"

# Shell completions
repoforge completions zsh > ~/.zfunc/_repoforge

# See exactly what would change — nothing is pushed
repoforge fix --user octocat

# Apply only safe, high-value fixes to repos scoring 70 or below
repoforge fix --user octocat --apply --only license,gitignore,ci --max-score 70

# Same, but open a reviewable pull request per repo instead of committing to main
repoforge fix --user octocat --pr --only license,gitignore
```

## The rubric

| Check | Weight | Passes when |
|-------|------:|-------------|
| README present | 12 | a README exists |
| README depth | 13 | ≥ 800 chars and ≥ 3 headings |
| License | 12 | a recognised license or `LICENSE` file |
| CI workflow | 12 | a `.github/workflows/*.yml` exists |
| Tests | 10 | test files/dirs detected for the language |
| Description | 8 | non-empty, ≥ 10 chars |
| Recent activity | 8 | pushed within the last 365 days |
| Topics | 7 | ≥ 3 topics set |
| Structure | 7 | `src/`-style layout, or markup + assets |
| `.gitignore` | 6 | present |
| Homepage/demo | 5 | a URL is set |

Weights and grade thresholds are overridable in `repoforge.toml` — they need not sum to 100; scores are normalised against their total.

## What the fixer will and won't do

Auto-generated: README scaffold, MIT license, language-aware `.gitignore` (Rust, Python, JS/TS, Go, Java/Kotlin, C/C++, C#, Ruby, PHP, Swift), language-aware CI workflow (Rust, Python, JS/TS, Go, Java, C/C++), derived topics, and a placeholder description. Everything is grounded in observed facts (language, file tree, owner) so the tool never fabricates claims about what a project does.

Not auto-generated: tests, real activity, and human-written overview prose — those are flagged but left to you, because faking them would be exactly the "fluff" this tool exists to remove.

## Structure

```
src/
  main.rs        CLI orchestration, token resolution, concurrency
  cli.rs         clap command/flag definitions
  github.rs      async REST client + data models
  audit.rs       the rubric: weighted checks, scoring, grading
  remediate.rs   fix generators (pure) + apply logic
  report.rs      table / summary / markdown / json output
  config.rs      rubric weights + thresholds (TOML-overridable)
tests/
  logic.rs       scoring + generators, network-free
```

## Changelog

- **0.9.0** — `audit --fail-under <score>`: exit non-zero when any repo scores below the threshold, so repoforge can gate CI.
- **0.8.0** — `completions` command: generate shell completion scripts (bash, zsh, fish, powershell, elvish).
- **0.7.1** — The topics fix now *merges* with existing topics instead of replacing them, so applying it can never delete tags you set by hand.
- **0.7.0** — `init` command: create a new repository already scaffolded to an A grade (README, LICENSE, language-aware `.gitignore` + CI, derived topics, description) in one shot.
- **0.6.1** — Live GitHub Pages dashboard (`docs/index.html`); the scheduled self-audit workflow now refreshes both the markdown report and the HTML dashboard each week.
- **0.6.0** — `badge` command: emit a ready-to-paste shields.io quality badge (markdown) or endpoint JSON for one or many repos. (repoforge grades itself **A**.)
- **0.5.0** — Writes (PUT/PATCH/POST) now retry with backoff on 429, 5xx, and GitHub's 403 + `Retry-After` *secondary* rate-limit response, so large `fix` sweeps don't half-fail.
- **0.4.0** — `--holder` flag to set the LICENSE copyright holder (defaults to the repo owner's login).
- **0.3.0** — `--pr` mode: apply file fixes via a reviewable pull request on a `repoforge/quality-fixes` branch instead of committing to the default branch.
- **0.2.0** — HTML dashboard report format; CI generators for Java and C/C++; `.gitignore` for C#, Ruby, PHP, Swift, Kotlin; GET retry with exponential backoff + `Retry-After`.
- **0.1.0** — Initial release: 11-check rubric, five output formats, fix generators, concurrent auditing, scheduled self-audit workflow.

## License

MIT — see [LICENSE](LICENSE).
