//! Rendering: the human report, the `--list` view and the JSON document.
//!
//! Every renderer is a pure function of its input and wraps at a fixed width
//! rather than the terminal width, so identical input produces identical bytes
//! on every machine and in every CI run.

use crate::catalog::{Baseline, Catalog, Feature, Provenance};
use crate::json::quote;
use crate::model::{Report, Verdict, CAVEAT};
use crate::target::Target;

/// Fixed wrap width. Not terminal-derived, on purpose.
const WRAP: usize = 92;

/// Column the detail fields' values start at.
const FIELD_WIDTH: usize = 13;

/// The four verdicts, in the order their sections are printed.
const SECTIONS: [Verdict; 4] = [
    Verdict::Risk,
    Verdict::Removable,
    Verdict::RemovableAtTarget,
    Verdict::Info,
];

/// Rendering options for the human output.
#[derive(Debug, Clone, Copy, Default)]
pub struct RenderOptions {
    /// Emit ANSI color codes.
    pub color: bool,
}

fn paint(opts: RenderOptions, code: &str, s: &str) -> String {
    if opts.color {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

/// "Widely available (newly 2023-12-19, widely 2026-06-19)".
pub fn baseline_phrase(feature: &Feature) -> String {
    let mut out = feature.baseline.label().to_string();
    let mut dates = Vec::new();
    if let Some(low) = feature.baseline_low_date {
        dates.push(format!("newly {low}"));
    }
    if let Some(high) = feature.baseline_high_date {
        dates.push(format!("widely {high}"));
    }
    if !dates.is_empty() {
        out.push_str(&format!(" ({})", dates.join(", ")));
    }
    out
}

/// How the run's target was arrived at, for the header and the footer.
fn target_phrase(report: &Report) -> String {
    match &report.target_source {
        Some(source) => format!(
            "{} [from {}: \"{}\"]",
            report.target.describe(),
            source.path,
            source.query
        ),
        None => report.target.describe(),
    }
}

// ---------------------------------------------------------------------------
// Human report
// ---------------------------------------------------------------------------

/// The full human-readable report, including the trailing caveat.
pub fn render_text(report: &Report, opts: RenderOptions) -> String {
    let mut out = format!("baselinefit: {}\n", report.root_name);
    out.push_str(&summary_line(report, opts));
    out.push('\n');
    out.push_str(&format!("target: {}\n", target_phrase(report)));
    out.push('\n');

    if report.groups.is_empty() {
        out.push_str(
            "Nothing to report: no feature in the snapshot is used unsafely for this target, and \
             no removable safety net was found.\n\n",
        );
    }

    for verdict in SECTIONS {
        let mut groups = report.groups_of(verdict).peekable();
        if groups.peek().is_none() {
            continue;
        }
        out.push_str(&paint(opts, "1", verdict.heading()));
        out.push('\n');
        for found in groups {
            let feature = &found.feature;
            out.push_str(&format!(
                "[{}] {}  {}\n",
                paint(opts, verdict.color(), verdict.label()),
                paint(opts, "1", &feature.id),
                feature.name,
            ));
            field(&mut out, "baseline", &baseline_phrase(feature));
            field(&mut out, "action", found.action());
            if let Some(spec) = &feature.spec {
                field(&mut out, "spec", spec);
            }
            if !feature.caniuse.is_empty() {
                field(&mut out, "caniuse", &feature.caniuse.join(", "));
            }
            out.push_str(&format!(
                "  {} finding(s) in {} file(s)\n",
                found.total,
                found.files.len()
            ));

            for file in &found.files {
                out.push_str(&format!("  {}\n", paint(opts, "4", &file.path)));
                for line in &file.lines {
                    out.push_str(&format!(
                        "    {}:{}  {}\n",
                        line.line, line.column, line.snippet
                    ));
                    for d in &line.detectors {
                        out.push_str(&paint(
                            opts,
                            "2",
                            &format!(
                                "      matched {} [{}] -- {}\n",
                                d.text,
                                d.kind.as_str(),
                                d.what
                            ),
                        ));
                        if let Some(action) = &d.action {
                            out.push_str(&paint(opts, "2", &format!("      {action}\n")));
                        }
                    }
                }
            }
            out.push('\n');
        }
    }

    out.push_str(&footer(report));
    out
}

/// The one-line summary used as the `--quiet` output and as the report header.
pub fn summary_line(report: &Report, opts: RenderOptions) -> String {
    let risk = report.count_of(Verdict::Risk);
    let removable =
        report.count_of(Verdict::Removable) + report.count_of(Verdict::RemovableAtTarget);
    let info = report.count_of(Verdict::Info);
    let headline = if risk > 0 {
        paint(opts, Verdict::Risk.color(), &format!("{risk} risk"))
    } else if removable > 0 {
        paint(opts, Verdict::Removable.color(), "no risk")
    } else {
        paint(opts, "32", "clean")
    };
    format!(
        "target {}  |  {}  |  {} risk, {} removable, {} info across {} file(s); \
         {} of {} feature(s) matched",
        report.target.as_str(),
        headline,
        risk,
        removable,
        info,
        report.files_with_findings(),
        report.features_matched(),
        report.features_searched,
    )
}

fn footer(report: &Report) -> String {
    let mut out = format!(
        "{} file(s) scanned, {} skipped ({} not a source extension, {} too large, {} binary, \
{} unreadable).\n",
        report.stats.scanned,
        report.stats.skipped_total(),
        report.stats.skipped_extension,
        report.stats.skipped_large,
        report.stats.skipped_binary,
        report.stats.skipped_unreadable,
    );
    out.push_str(&provenance_lines(&report.dataset));
    for line in wrap(CAVEAT, WRAP) {
        out.push_str(&line);
        out.push('\n');
    }
    out
}

fn provenance_lines(dataset: &Provenance) -> String {
    format!(
        "{}\nSource: {} (sha256 {})\n",
        dataset.credit(),
        dataset.source_url,
        dataset.sha256
    )
}

/// A `  label       value` line, wrapped with a hanging indent.
fn field(out: &mut String, label: &str, value: &str) {
    let width = FIELD_WIDTH;
    let indent = " ".repeat(FIELD_WIDTH + 2);
    for (i, line) in wrap(value, WRAP - FIELD_WIDTH - 2).into_iter().enumerate() {
        if i == 0 {
            out.push_str(&format!("  {label:<width$}{line}\n"));
        } else {
            out.push_str(&format!("{indent}{line}\n"));
        }
    }
}

/// Greedy word wrap. Words longer than `width` are left intact rather than
/// broken, because they are almost always URLs.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word.chars().count() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

// ---------------------------------------------------------------------------
// --list
// ---------------------------------------------------------------------------

/// Marker shown in `--list` for a feature that has reached the target.
fn reach(target: Target, feature: &Feature) -> &'static str {
    if target.satisfied_by(feature) {
        "reached"
    } else {
        "not yet"
    }
}

/// The `--list` view: every feature in the snapshot, its Baseline status, and
/// whether it has reached the target.
pub fn render_list_text(catalog: &Catalog, target: Target, opts: RenderOptions) -> String {
    let mut out = format!(
        "baselinefit catalog: {} feature(s), {} detector(s)\ntarget: {}\n\n",
        catalog.features.len(),
        catalog.detector_count(),
        target.describe(),
    );
    let id_width = catalog
        .features
        .iter()
        .map(|f| f.id.chars().count())
        .max()
        .unwrap_or(2)
        .max(2);

    out.push_str(&format!(
        "  {:<7}  {:<8}  {:<width$}  {:<12}  {}\n",
        "STATUS",
        "TARGET",
        "ID",
        "SINCE",
        "NAME",
        width = id_width
    ));
    for feature in &catalog.features {
        let since = match feature.milestone() {
            Some((_, date)) => date.to_string(),
            None => "-".to_string(),
        };
        // Pad before painting, or the escape codes would count toward the
        // column width and break the alignment.
        let status = paint(
            opts,
            baseline_color(feature.baseline),
            feature.baseline.short(),
        );
        out.push_str(&format!(
            "  {}  {:<8}  {:<width$}  {:<12}  {}\n",
            status,
            reach(target, feature),
            feature.id,
            since,
            feature.name,
            width = id_width
        ));
    }
    out.push('\n');

    for feature in &catalog.features {
        out.push_str(&format!("{}\n", paint(opts, "1", &feature.id)));
        field(&mut out, "name", &feature.name);
        field(&mut out, "baseline", &baseline_phrase(feature));
        field(
            &mut out,
            "at target",
            &format!("{} ({})", reach(target, feature), target.as_str()),
        );
        if let Some(action) = &feature.risk_action {
            field(&mut out, "if risky", action);
        }
        if let Some(action) = &feature.removable_action {
            field(&mut out, "if removable", action);
        }
        if let Some(spec) = &feature.spec {
            field(&mut out, "spec", spec);
        }
        if !feature.caniuse.is_empty() {
            field(&mut out, "caniuse", &feature.caniuse.join(", "));
        }
        let detectors: Vec<String> = feature
            .detectors
            .iter()
            .map(|d| format!("{} [{} {}]", d.text, d.direction.as_str(), d.kind.as_str()))
            .collect();
        field(&mut out, "detectors", &detectors.join(", "));
        out.push('\n');
    }

    out.push_str(&provenance_lines(&catalog.dataset));
    for line in wrap(CAVEAT, WRAP) {
        out.push_str(&line);
        out.push('\n');
    }
    out
}

fn baseline_color(baseline: Baseline) -> &'static str {
    match baseline {
        Baseline::Limited => "31",
        Baseline::Newly => "33",
        Baseline::Widely => "32",
    }
}

/// The `--list --json` document.
pub fn render_list_json(catalog: &Catalog, target: Target) -> String {
    let mut out = String::with_capacity(16384);
    out.push_str("{\n");
    out.push_str("  \"tool\": \"baselinefit\",\n");
    out.push_str(&format!(
        "  \"version\": {},\n",
        quote(env!("CARGO_PKG_VERSION"))
    ));
    out.push_str(&format!(
        "  \"schema\": {},\n",
        crate::catalog::SCHEMA_VERSION
    ));
    out.push_str(&format!("  \"target\": {},\n", target_json(target, None)));
    out.push_str(&format!(
        "  \"dataset\": {},\n",
        dataset_json(&catalog.dataset)
    ));
    out.push_str("  \"features\": [\n");
    for (i, feature) in catalog.features.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&feature_json_body(feature, "      ", true));
        out.push_str(&format!(
            "      \"reaches_target\": {},\n",
            target.satisfied_by(feature)
        ));
        out.push_str(&format!(
            "      \"risk_action\": {},\n",
            opt(feature.risk_action.as_deref())
        ));
        out.push_str(&format!(
            "      \"removable_action\": {},\n",
            opt(feature.removable_action.as_deref())
        ));
        out.push_str("      \"detectors\": [\n");
        for (j, d) in feature.detectors.iter().enumerate() {
            let scope: Vec<String> = d.scope.iter().map(|g| quote(g)).collect();
            out.push_str(&format!(
                "        {{\"direction\": {}, \"when\": {}, \"kind\": {}, \"text\": {}, \
\"what\": {}, \"scope\": [{}]}}",
                quote(d.direction.as_str()),
                quote(d.when.as_str()),
                quote(d.kind.as_str()),
                quote(&d.text),
                quote(&d.what),
                scope.join(", ")
            ));
            out.push_str(sep(j, feature.detectors.len()));
        }
        out.push_str("      ]\n");
        out.push_str("    }");
        out.push_str(sep(i, catalog.features.len()));
    }
    out.push_str("  ],\n");
    out.push_str(&format!("  \"caveat\": {}\n", quote(CAVEAT)));
    out.push_str("}\n");
    out
}

// ---------------------------------------------------------------------------
// JSON report
// ---------------------------------------------------------------------------

/// The machine-readable report. The shape is documented in README.md.
pub fn render_json(report: &Report) -> String {
    let mut out = String::with_capacity(16384);
    out.push_str("{\n");
    out.push_str("  \"tool\": \"baselinefit\",\n");
    out.push_str(&format!(
        "  \"version\": {},\n",
        quote(env!("CARGO_PKG_VERSION"))
    ));
    out.push_str(&format!(
        "  \"schema\": {},\n",
        crate::catalog::SCHEMA_VERSION
    ));
    out.push_str(&format!("  \"root\": {},\n", quote(&report.root_name)));
    out.push_str(&format!(
        "  \"target\": {},\n",
        target_json(report.target, report.target_source.as_ref())
    ));
    out.push_str(&format!(
        "  \"dataset\": {},\n",
        dataset_json(&report.dataset)
    ));
    out.push_str(&format!(
        "  \"features_searched\": {},\n",
        report.features_searched
    ));
    out.push_str(&format!(
        "  \"totals\": {{\"findings\": {}, \"risk\": {}, \"removable\": {}, \
\"removable_at_target\": {}, \"info\": {}, \"files_with_findings\": {}, \
\"features_matched\": {}}},\n",
        report.total_findings(),
        report.count_of(Verdict::Risk),
        report.count_of(Verdict::Removable),
        report.count_of(Verdict::RemovableAtTarget),
        report.count_of(Verdict::Info),
        report.files_with_findings(),
        report.features_matched(),
    ));
    out.push_str(&format!(
        "  \"stats\": {{\"scanned\": {}, \"skipped_extension\": {}, \"skipped_large\": {}, \
\"skipped_binary\": {}, \"skipped_unreadable\": {}}},\n",
        report.stats.scanned,
        report.stats.skipped_extension,
        report.stats.skipped_large,
        report.stats.skipped_binary,
        report.stats.skipped_unreadable,
    ));

    out.push_str("  \"findings\": [\n");
    for (i, found) in report.groups.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str("      \"feature\": {\n");
        out.push_str(&feature_json_body(&found.feature, "        ", false));
        out.push_str("      },\n");
        out.push_str(&format!(
            "      \"direction\": {},\n",
            quote(found.direction.as_str())
        ));
        out.push_str(&format!(
            "      \"verdict\": {},\n",
            quote(found.verdict.as_str())
        ));
        out.push_str(&format!(
            "      \"reaches_target\": {},\n",
            found.satisfies_target
        ));
        out.push_str(&format!("      \"action\": {},\n", quote(found.action())));
        out.push_str(&format!("      \"count\": {},\n", found.total));
        out.push_str("      \"files\": [\n");
        for (j, file) in found.files.iter().enumerate() {
            out.push_str("        {\n");
            out.push_str(&format!("          \"path\": {},\n", quote(&file.path)));
            out.push_str(&format!("          \"count\": {},\n", file.lines.len()));
            out.push_str("          \"lines\": [\n");
            for (k, line) in file.lines.iter().enumerate() {
                let detectors: Vec<String> = line
                    .detectors
                    .iter()
                    .map(|d| {
                        format!(
                            "{{\"text\": {}, \"kind\": {}, \"what\": {}, \"column\": {}}}",
                            quote(&d.text),
                            quote(d.kind.as_str()),
                            quote(&d.what),
                            d.column
                        )
                    })
                    .collect();
                out.push_str(&format!(
                    "            {{\"line\": {}, \"column\": {}, \"snippet\": {}, \
\"detectors\": [{}]}}",
                    line.line,
                    line.column,
                    quote(&line.snippet),
                    detectors.join(", ")
                ));
                out.push_str(sep(k, file.lines.len()));
            }
            out.push_str("          ]\n");
            out.push_str("        }");
            out.push_str(sep(j, found.files.len()));
        }
        out.push_str("      ]\n");
        out.push_str("    }");
        out.push_str(sep(i, report.groups.len()));
    }
    out.push_str("  ],\n");
    out.push_str(&format!("  \"caveat\": {}\n", quote(CAVEAT)));
    out.push_str("}\n");
    out
}

fn target_json(target: Target, source: Option<&crate::target::BrowserslistSource>) -> String {
    let source = match source {
        Some(s) => format!(
            "{{\"path\": {}, \"query\": {}}}",
            quote(&s.path),
            quote(&s.query)
        ),
        None => "null".to_string(),
    };
    format!(
        "{{\"level\": {}, \"browserslist_query\": {}, \"description\": {}, \"source\": {}}}",
        quote(&target.as_str()),
        quote(&target.browserslist_query()),
        quote(&target.describe()),
        source
    )
}

fn dataset_json(dataset: &crate::catalog::Provenance) -> String {
    format!(
        "{{\"package\": {}, \"version\": {}, \"source_url\": {}, \"sha256\": {}, \
\"retrieved_on\": {}, \"generator\": {}}}",
        quote(&dataset.package),
        quote(&dataset.version),
        quote(&dataset.source_url),
        quote(&dataset.sha256),
        quote(&dataset.retrieved_on.to_string()),
        quote(&dataset.generator),
    )
}

/// The dataset-derived members of a feature, one per line, indented by
/// `indent`. With `trailing_comma`, the last member ends in `,` so the caller
/// can append further members to the same object.
fn feature_json_body(feature: &Feature, indent: &str, trailing_comma: bool) -> String {
    let tail = if trailing_comma { ",\n" } else { "\n" };
    let caniuse: Vec<String> = feature.caniuse.iter().map(|c| quote(c)).collect();
    let mut out = String::new();
    out.push_str(&format!("{indent}\"id\": {},\n", quote(&feature.id)));
    out.push_str(&format!("{indent}\"name\": {},\n", quote(&feature.name)));
    out.push_str(&format!(
        "{indent}\"description\": {},\n",
        quote(&feature.description)
    ));
    out.push_str(&format!(
        "{indent}\"baseline\": {},\n",
        quote(feature.baseline.as_dataset_str())
    ));
    out.push_str(&format!(
        "{indent}\"baseline_status\": {},\n",
        quote(feature.baseline.label())
    ));
    out.push_str(&format!(
        "{indent}\"baseline_low_date\": {},\n",
        opt_date(feature.baseline_low_date)
    ));
    out.push_str(&format!(
        "{indent}\"baseline_high_date\": {},\n",
        opt_date(feature.baseline_high_date)
    ));
    out.push_str(&format!(
        "{indent}\"spec\": {},\n",
        opt(feature.spec.as_deref())
    ));
    out.push_str(&format!(
        "{indent}\"caniuse\": [{}]{tail}",
        caniuse.join(", ")
    ));
    out
}

/// The separator after a JSON array element: a comma unless it is the last.
fn sep(index: usize, len: usize) -> &'static str {
    if index + 1 < len {
        ",\n"
    } else {
        "\n"
    }
}

fn opt(value: Option<&str>) -> String {
    match value {
        Some(s) => quote(s),
        None => "null".to_string(),
    }
}

fn opt_date(value: Option<crate::date::Date>) -> String {
    match value {
        Some(d) => quote(&d.to_string()),
        None => "null".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Direction, MatchKind};
    use crate::matcher::{Hit, MatchedDetector};
    use crate::model::{Report, ScanStats};
    use crate::target::BrowserslistSource;

    fn index_of(catalog: &Catalog, id: &str) -> usize {
        catalog.features.iter().position(|f| f.id == id).unwrap()
    }

    fn hit(
        catalog: &Catalog,
        id: &str,
        direction: Direction,
        text: &str,
        kind: MatchKind,
        what: &str,
    ) -> Hit {
        Hit {
            feature_index: index_of(catalog, id),
            direction,
            line: 12,
            snippet: text.to_string(),
            detectors: vec![MatchedDetector {
                text: text.to_string(),
                kind,
                what: what.to_string(),
                action: None,
                column: 3,
            }],
        }
    }

    /// The three shapes a report can carry: a risk, a clean removal, and a
    /// forward pointer that must never read as a removal.
    fn all_hits(catalog: &Catalog) -> Vec<crate::matcher::FileHits> {
        vec![(
            "src/a.css".to_string(),
            vec![
                hit(
                    catalog,
                    "container-queries",
                    Direction::Risk,
                    "@container",
                    MatchKind::AtRule,
                    "container query at-rule",
                ),
                hit(
                    catalog,
                    "box-shadow",
                    Direction::Removable,
                    "-webkit-box-shadow",
                    MatchKind::Property,
                    "vendor prefix",
                ),
                hit(
                    catalog,
                    "anchor-positioning",
                    Direction::Removable,
                    "@floating-ui/",
                    MatchKind::Package,
                    "JavaScript positioning library",
                ),
            ],
        )]
    }

    fn build(target: Target, hits: Vec<crate::matcher::FileHits>) -> Report {
        Report::build(
            "demo".into(),
            target,
            None,
            &Catalog::builtin(),
            hits,
            ScanStats {
                scanned: 3,
                skipped_extension: 1,
                ..ScanStats::default()
            },
        )
    }

    fn report(target: Target, with_hits: bool) -> Report {
        let catalog = Catalog::builtin();
        let hits = if with_hits {
            all_hits(&catalog)
        } else {
            vec![]
        };
        build(target, hits)
    }

    #[test]
    fn the_text_report_carries_every_required_element() {
        let text = render_text(&report(Target::Year(2020), true), RenderOptions::default());
        for needle in [
            "baselinefit: demo",
            "target 2020",
            "Baseline 2020 (Newly available in 2020 or earlier)",
            "RISK -- used, but not safe for your target yet",
            "container-queries",
            "Widely available (newly 2023-02-14, widely 2025-08-14)",
            "src/a.css",
            "12:3  @container",
            "matched @container [at-rule] -- container query at-rule",
            "REMOVABLE",
            "INFO -- pointers only, nothing to delete yet",
            "Baseline data: web-features@",
            "retrieved 2026-11-11",
            "sha256",
            "textual, not semantic",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
        }
    }

    #[test]
    fn the_report_never_says_delete_about_a_feature_that_has_not_arrived() {
        let text = render_text(&report(Target::Widely, true), RenderOptions::default());
        // The Floating UI pointer is filed under INFO with the "do not remove"
        // wording, because anchor positioning is not Baseline yet.
        let info = text.split("INFO --").nth(1).expect("an INFO section");
        assert!(info.contains("anchor-positioning"), "{info}");
        assert!(info.contains("Do not remove"), "{info}");
    }

    #[test]
    fn a_clean_report_says_so_and_still_prints_provenance() {
        let text = render_text(&report(Target::Widely, false), RenderOptions::default());
        assert!(text.contains("Nothing to report"));
        assert!(text.contains("Baseline data: web-features@"));
        assert!(!text.contains("RISK --"));
    }

    #[test]
    fn the_summary_line_leads_with_risk_then_cleanup_then_clean() {
        let opts = RenderOptions::default();
        assert!(summary_line(&report(Target::Year(2020), true), opts).contains("1 risk"));

        // Cleanup only: nothing is unsafe, but there is something to delete.
        let catalog = Catalog::builtin();
        let cleanup = vec![(
            "src/a.css".to_string(),
            vec![hit(
                &catalog,
                "box-shadow",
                Direction::Removable,
                "-webkit-box-shadow",
                MatchKind::Property,
                "vendor prefix",
            )],
        )];
        assert!(summary_line(&build(Target::Widely, cleanup), opts).contains("no risk"));

        assert!(summary_line(&report(Target::Widely, false), opts).contains("clean"));
    }

    #[test]
    fn a_browserslist_target_names_its_source_file() {
        let catalog = Catalog::builtin();
        let r = Report::build(
            "demo".into(),
            Target::Widely,
            Some(BrowserslistSource {
                path: ".browserslistrc".into(),
                query: "baseline widely available".into(),
            }),
            &catalog,
            vec![],
            ScanStats::default(),
        );
        let text = render_text(&r, RenderOptions::default());
        assert!(
            text.contains("[from .browserslistrc: \"baseline widely available\"]"),
            "{text}"
        );
        let doc = crate::json::parse(&render_json(&r)).unwrap();
        let source = doc.get("target").unwrap().get("source").unwrap();
        assert_eq!(
            source.get("path").unwrap().as_str(),
            Some(".browserslistrc")
        );
    }

    #[test]
    fn color_is_opt_in_and_never_leaks_when_off() {
        let r = report(Target::Year(2020), true);
        assert!(!render_text(&r, RenderOptions { color: false }).contains('\x1b'));
        assert!(render_text(&r, RenderOptions { color: true }).contains('\x1b'));
    }

    #[test]
    fn rendering_is_deterministic() {
        let r = report(Target::Year(2020), true);
        assert_eq!(
            render_text(&r, RenderOptions::default()),
            render_text(&r, RenderOptions::default())
        );
        assert_eq!(render_json(&r), render_json(&r));
    }

    #[test]
    fn the_json_report_parses_and_has_the_documented_shape() {
        let doc = crate::json::parse(&render_json(&report(Target::Year(2020), true))).unwrap();
        assert_eq!(doc.get("tool").unwrap().as_str(), Some("baselinefit"));
        let target = doc.get("target").unwrap();
        assert_eq!(target.get("level").unwrap().as_str(), Some("2020"));
        assert_eq!(
            target.get("browserslist_query").unwrap().as_str(),
            Some("baseline 2020")
        );
        assert!(target.get("source").unwrap().is_null());

        let dataset = doc.get("dataset").unwrap();
        assert_eq!(
            dataset.get("package").unwrap().as_str(),
            Some("web-features")
        );
        assert_eq!(
            dataset.get("retrieved_on").unwrap().as_str(),
            Some("2026-11-11")
        );

        let totals = doc.get("totals").unwrap();
        assert_eq!(totals.get("risk"), Some(&crate::json::Value::Number(1.0)));

        let findings = doc.get("findings").unwrap().as_array().unwrap();
        assert_eq!(findings.len(), 3);
        let first = &findings[0];
        assert_eq!(first.get("verdict").unwrap().as_str(), Some("risk"));
        assert_eq!(first.get("direction").unwrap().as_str(), Some("risk"));
        let feature = first.get("feature").unwrap();
        assert_eq!(
            feature.get("id").unwrap().as_str(),
            Some("container-queries")
        );
        assert_eq!(feature.get("baseline").unwrap().as_str(), Some("high"));
        assert_eq!(
            feature.get("baseline_high_date").unwrap().as_str(),
            Some("2025-08-14")
        );
        let files = first.get("files").unwrap().as_array().unwrap();
        assert_eq!(files[0].get("path").unwrap().as_str(), Some("src/a.css"));
        let lines = files[0].get("lines").unwrap().as_array().unwrap();
        assert_eq!(
            lines[0].get("line"),
            Some(&crate::json::Value::Number(12.0))
        );
        let detectors = lines[0].get("detectors").unwrap().as_array().unwrap();
        assert_eq!(detectors[0].get("kind").unwrap().as_str(), Some("at-rule"));
    }

    #[test]
    fn a_clean_json_report_still_parses() {
        let doc = crate::json::parse(&render_json(&report(Target::Widely, false))).unwrap();
        assert!(doc.get("findings").unwrap().as_array().unwrap().is_empty());
        assert_eq!(
            doc.get("totals").unwrap().get("findings"),
            Some(&crate::json::Value::Number(0.0))
        );
    }

    #[test]
    fn the_list_view_shows_status_dates_detectors_and_provenance() {
        let text = render_list_text(
            &Catalog::builtin(),
            Target::Widely,
            RenderOptions::default(),
        );
        assert!(text.contains("container-queries"));
        assert!(text.contains("2025-08-14"));
        assert!(text.contains("@container [risk at-rule]"));
        assert!(text.contains("Baseline data: web-features@"));
        assert!(text.contains("reached"));
        assert!(text.contains("not yet"));
        assert!(text.contains("textual, not semantic"));
    }

    #[test]
    fn the_list_json_parses_and_reports_target_reach() {
        let doc =
            crate::json::parse(&render_list_json(&Catalog::builtin(), Target::Widely)).unwrap();
        let features = doc.get("features").unwrap().as_array().unwrap();
        assert!(features.len() >= 40);
        let anchor = features
            .iter()
            .find(|f| f.get("id").unwrap().as_str() == Some("anchor-positioning"))
            .unwrap();
        assert_eq!(
            anchor.get("reaches_target"),
            Some(&crate::json::Value::Bool(false))
        );
        assert_eq!(anchor.get("baseline").unwrap().as_str(), Some("false"));
        assert!(anchor.get("baseline_low_date").unwrap().is_null());
        let detectors = anchor.get("detectors").unwrap().as_array().unwrap();
        assert!(!detectors.is_empty());
    }

    #[test]
    fn baseline_phrases_name_only_the_dates_the_dataset_has() {
        let catalog = Catalog::builtin();
        let by_id = |id: &str| catalog.features.iter().find(|f| f.id == id).unwrap();
        assert_eq!(
            baseline_phrase(by_id("has")),
            "Widely available (newly 2023-12-19, widely 2026-06-19)"
        );
        assert_eq!(
            baseline_phrase(by_id("backdrop-filter")),
            "Newly available (newly 2024-09-16)"
        );
        assert_eq!(
            baseline_phrase(by_id("anchor-positioning")),
            "Limited availability"
        );
    }

    #[test]
    fn actions_fall_back_rather_than_printing_nothing() {
        let catalog = Catalog::builtin();
        let user_select = catalog
            .features
            .iter()
            .find(|f| f.id == "user-select")
            .unwrap();
        assert!(user_select.action(Direction::Risk).len() > 20);
        assert!(user_select
            .action(Direction::Removable)
            .contains("case by case"));
    }

    #[test]
    fn wrap_respects_the_width_and_keeps_urls_intact() {
        assert_eq!(wrap("aaa bbb ccc ddd", 7), ["aaa bbb", "ccc ddd"]);
        let long = "https://example.com/a/very/long/path/that/exceeds/the/width";
        assert_eq!(wrap(long, 10), [long]);
        assert_eq!(wrap("", 10), [""]);
    }
}
