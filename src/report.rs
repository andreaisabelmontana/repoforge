//! Output formatting: a colored terminal table, an aggregate summary, a markdown report, and
//! machine-readable JSON.

use crate::audit::RepoAudit;
use colored::{Color, Colorize};
use comfy_table::{Cell, ContentArrangement, Table};
use std::collections::BTreeMap;

fn grade_color(g: char) -> Color {
    match g {
        'A' => Color::Green,
        'B' => Color::BrightGreen,
        'C' => Color::Yellow,
        'D' => Color::BrightRed,
        _ => Color::Red,
    }
}

/// Compact per-repo table sorted worst-first, so attention goes where it's needed.
pub fn table(audits: &[RepoAudit]) -> String {
    let mut sorted: Vec<&RepoAudit> = audits.iter().collect();
    sorted.sort_by(|a, b| a.score.cmp(&b.score).then(a.full_name.cmp(&b.full_name)));

    let mut table = Table::new();
    table
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec!["Repo", "Lang", "Score", "Grade", "Top gaps"]);

    for a in sorted {
        let gaps: Vec<&str> = a.failing().take(4).map(|c| c.label).collect();
        table.add_row(vec![
            Cell::new(&a.full_name),
            Cell::new(a.language.as_deref().unwrap_or("-")),
            Cell::new(format!("{:>3}", a.score)),
            Cell::new(a.grade.to_string()),
            Cell::new(if gaps.is_empty() {
                "—".to_string()
            } else {
                gaps.join(", ")
            }),
        ]);
    }
    table.to_string()
}

/// One-screen rollup: grade distribution, average score, and the most common missing pieces.
pub fn summary(audits: &[RepoAudit]) -> String {
    if audits.is_empty() {
        return "no repositories audited".into();
    }
    let n = audits.len();
    let avg = audits.iter().map(|a| a.score as usize).sum::<usize>() as f64 / n as f64;

    let mut dist: BTreeMap<char, usize> = BTreeMap::new();
    for a in audits {
        *dist.entry(a.grade).or_default() += 1;
    }

    let mut gap_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for a in audits {
        for c in a.failing() {
            *gap_counts.entry(c.label).or_default() += 1;
        }
    }
    let mut gaps: Vec<(&str, usize)> = gap_counts.into_iter().collect();
    gaps.sort_by_key(|g| std::cmp::Reverse(g.1));

    let mut out = String::new();
    out.push_str(&format!("\n{}\n", "── Summary ──".bold()));
    out.push_str(&format!("Repositories : {n}\n"));
    out.push_str(&format!("Average score: {avg:.1}/100\n"));

    out.push_str("Grades       : ");
    for g in ['A', 'B', 'C', 'D', 'F'] {
        let c = dist.get(&g).copied().unwrap_or(0);
        out.push_str(&format!("{} ", format!("{g}:{c}").color(grade_color(g))));
    }
    out.push('\n');

    out.push_str(&format!("\n{}\n", "Most common gaps:".bold()));
    for (label, count) in gaps.iter().take(8) {
        let pct = (count * 100) / n;
        out.push_str(&format!("  {label:<16} {count:>4} repos ({pct}%)\n"));
    }
    out
}

/// Full markdown report suitable for committing to a repo or pasting into an issue.
pub fn markdown(audits: &[RepoAudit]) -> String {
    let mut sorted: Vec<&RepoAudit> = audits.iter().collect();
    sorted.sort_by(|a, b| a.score.cmp(&b.score).then(a.full_name.cmp(&b.full_name)));

    let n = sorted.len().max(1);
    let avg = sorted.iter().map(|a| a.score as usize).sum::<usize>() as f64 / n as f64;

    let mut s = String::new();
    s.push_str("# Repository quality report\n\n");
    s.push_str(&format!("- Repositories audited: **{}**\n", sorted.len()));
    s.push_str(&format!("- Average score: **{avg:.1}/100**\n\n"));
    s.push_str("| Repo | Lang | Score | Grade | Top gaps |\n");
    s.push_str("|------|------|------:|:-----:|----------|\n");
    for a in &sorted {
        let gaps: Vec<&str> = a.failing().take(5).map(|c| c.label).collect();
        s.push_str(&format!(
            "| `{}` | {} | {} | {} | {} |\n",
            a.full_name,
            a.language.as_deref().unwrap_or("—"),
            a.score,
            a.grade,
            if gaps.is_empty() {
                "—".into()
            } else {
                gaps.join(", ")
            },
        ));
    }
    s
}

pub fn json(audits: &[RepoAudit]) -> String {
    serde_json::to_string_pretty(audits).unwrap_or_else(|_| "[]".into())
}
