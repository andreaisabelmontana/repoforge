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

fn shields_color(g: char) -> &'static str {
    match g {
        'A' => "brightgreen",
        'B' => "green",
        'C' => "yellow",
        'D' => "orange",
        _ => "red",
    }
}

/// A ready-to-paste shields.io badge (static) showing a repo's grade and score.
pub fn badge_markdown(a: &RepoAudit) -> String {
    let msg = format!("{} ({})", a.grade, a.score);
    // shields static-badge encoding: space -> %20, ( -> %28, ) -> %29, literal - -> --
    let enc = msg
        .replace('-', "--")
        .replace(' ', "%20")
        .replace('(', "%28")
        .replace(')', "%29");
    format!(
        "![repoforge quality](https://img.shields.io/badge/repoforge-{enc}-{})",
        shields_color(a.grade)
    )
}

/// shields.io "endpoint" schema JSON — host it (repo file / gist) and reference via
/// `https://img.shields.io/endpoint?url=<raw-url>` for a badge that updates with the file.
pub fn badge_endpoint(a: &RepoAudit) -> String {
    serde_json::json!({
        "schemaVersion": 1,
        "label": "repoforge",
        "message": format!("{} ({})", a.grade, a.score),
        "color": shields_color(a.grade),
    })
    .to_string()
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn html_grade_color(g: char) -> &'static str {
    match g {
        'A' => "#1a7f37",
        'B' => "#3fa34d",
        'C' => "#bf8700",
        'D' => "#d4731f",
        _ => "#cf222e",
    }
}

/// A self-contained HTML dashboard (inline CSS, no assets) — open in a browser or publish to Pages.
pub fn html(audits: &[RepoAudit]) -> String {
    let mut sorted: Vec<&RepoAudit> = audits.iter().collect();
    sorted.sort_by(|a, b| a.score.cmp(&b.score).then(a.full_name.cmp(&b.full_name)));

    let n = sorted.len().max(1);
    let avg = sorted.iter().map(|a| a.score as usize).sum::<usize>() as f64 / n as f64;

    let mut dist: BTreeMap<char, usize> = BTreeMap::new();
    for a in &sorted {
        *dist.entry(a.grade).or_default() += 1;
    }

    let mut rows = String::new();
    for a in &sorted {
        let gaps: Vec<&str> = a.failing().take(6).map(|c| c.label).collect();
        let gaps_s = if gaps.is_empty() {
            "—".to_string()
        } else {
            esc(&gaps.join(", "))
        };
        let color = html_grade_color(a.grade);
        rows.push_str(&format!(
            "<tr><td><a href=\"https://github.com/{full}\">{full}</a></td>\
<td class=\"lang\">{lang}</td>\
<td class=\"score\"><span class=\"bar\" style=\"--w:{score}%;--c:{color}\"></span>{score}</td>\
<td class=\"grade\" style=\"color:{color}\">{grade}</td>\
<td class=\"gaps\">{gaps_s}</td></tr>",
            full = esc(&a.full_name),
            lang = esc(a.language.as_deref().unwrap_or("—")),
            score = a.score,
            grade = a.grade,
        ));
    }

    let mut bars = String::new();
    for g in ['A', 'B', 'C', 'D', 'F'] {
        let c = dist.get(&g).copied().unwrap_or(0);
        let pct = (c * 100) / n;
        bars.push_str(&format!(
            "<div class=\"gbar\"><span class=\"glabel\" style=\"color:{col}\">{g}</span>\
<span class=\"gtrack\"><span class=\"gfill\" style=\"width:{pct}%;background:{col}\"></span></span>\
<span class=\"gcount\">{c}</span></div>",
            col = html_grade_color(g),
        ));
    }

    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<title>Repository quality report</title><style>\
:root{{color-scheme:light dark}}\
*{{box-sizing:border-box}}\
body{{font:15px/1.5 ui-sans-serif,system-ui,-apple-system,Segoe UI,Roboto,sans-serif;margin:0;background:#fafafa;color:#1c1c1c}}\
@media(prefers-color-scheme:dark){{body{{background:#0d1117;color:#e6edf3}}.card,thead th{{background:#161b22 !important}}tr:hover td{{background:#161b22 !important}}a{{color:#58a6ff}}}}\
.wrap{{max-width:1000px;margin:0 auto;padding:40px 24px}}\
h1{{font-size:26px;margin:0 0 4px}}\
.sub{{color:#777;margin:0 0 28px}}\
.stats{{display:flex;gap:28px;flex-wrap:wrap;margin-bottom:28px}}\
.card{{background:#fff;border:1px solid #e3e3e3;border-radius:12px;padding:18px 22px}}\
.big{{font-size:34px;font-weight:700;line-height:1}}\
.cap{{color:#777;font-size:13px;text-transform:uppercase;letter-spacing:.5px;margin-top:6px}}\
.dist{{min-width:280px}}\
.gbar{{display:flex;align-items:center;gap:10px;margin:4px 0}}\
.glabel{{width:14px;font-weight:700}}\
.gtrack{{flex:1;height:9px;background:#eee;border-radius:5px;overflow:hidden}}\
.gfill{{display:block;height:100%}}\
.gcount{{width:34px;text-align:right;color:#777;font-variant-numeric:tabular-nums}}\
table{{width:100%;border-collapse:collapse;background:#fff;border:1px solid #e3e3e3;border-radius:12px;overflow:hidden}}\
thead th{{text-align:left;background:#f3f3f3;padding:10px 14px;font-size:12px;text-transform:uppercase;letter-spacing:.4px;color:#666}}\
td{{padding:10px 14px;border-top:1px solid #efefef;vertical-align:middle}}\
tr:hover td{{background:#f7f7f7}}\
.lang{{color:#777;white-space:nowrap}}\
.grade{{font-weight:700;text-align:center}}\
.score{{position:relative;width:120px;font-variant-numeric:tabular-nums}}\
.bar{{position:absolute;left:0;bottom:0;height:3px;width:var(--w);background:var(--c);opacity:.7}}\
.gaps{{color:#666;font-size:13px}}\
a{{color:#0969da;text-decoration:none}}a:hover{{text-decoration:underline}}\
footer{{color:#999;font-size:12px;margin-top:24px;text-align:center}}\
</style></head><body><div class=\"wrap\">\
<h1>Repository quality report</h1>\
<p class=\"sub\">{n} repositories &middot; graded by repoforge</p>\
<div class=\"stats\">\
<div class=\"card\"><div class=\"big\">{avg:.1}</div><div class=\"cap\">avg / 100</div></div>\
<div class=\"card\"><div class=\"big\">{n}</div><div class=\"cap\">repositories</div></div>\
<div class=\"card dist\">{bars}</div>\
</div>\
<table><thead><tr><th>Repository</th><th>Lang</th><th>Score</th><th>Grade</th><th>Top gaps</th></tr></thead>\
<tbody>{rows}</tbody></table>\
<footer>Generated by repoforge · worst-first</footer>\
</div></body></html>"
    )
}
