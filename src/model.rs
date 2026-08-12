//! The report model: how raw hits become a grouped, ordered, judged report.
//!
//! Pure data plus the rollup. Nothing here touches the filesystem, so the whole
//! reporting contract -- grouping, ordering, verdicts, exit codes -- is
//! unit-testable from in-memory inputs.

use std::fmt;

use crate::catalog::{Baseline, Catalog, Direction, Feature, Provenance};
use crate::matcher::{FileHits, MatchedDetector};
use crate::target::{BrowserslistSource, Target};

/// The framing printed under every report and every `--list`.
pub const CAVEAT: &str = "Baseline statuses and dates are copied from the embedded web-features \
snapshot named above. It is a snapshot: re-run tools/fetch-data.py to refresh it, and check the \
live data before a decision that matters. Matching is textual, not semantic: baselinefit does not \
parse CSS or JavaScript, so hits in comments, strings, documentation and vendored code are \
reported like any other.";

/// What baselinefit concluded about one feature in one direction.
///
/// The distinction between [`Verdict::Removable`] and
/// [`Verdict::RemovableAtTarget`] is the point of the type. "Delete this" is
/// only an unqualified statement when the dataset says the feature is Baseline
/// Widely available. When a looser `--target` is what made the safety net
/// redundant, the report says so instead of pretending otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    /// The feature has not reached the target. Using it is a risk.
    Risk,
    /// The feature is Baseline Widely available and the target permits it.
    /// The safety net is dead weight.
    Removable,
    /// The feature satisfies the chosen target but is not yet Widely
    /// available. Removable for a project that has accepted this target, and
    /// not a general recommendation.
    RemovableAtTarget,
    /// A pointer at a replacement that is not ready. Nothing to delete.
    Info,
}

impl Verdict {
    /// Classify a finding.
    pub fn of(direction: Direction, satisfied: bool, baseline: Baseline) -> Verdict {
        match (direction, satisfied) {
            (Direction::Risk, _) => Verdict::Risk,
            (Direction::Removable, false) => Verdict::Info,
            (Direction::Removable, true) if baseline == Baseline::Widely => Verdict::Removable,
            (Direction::Removable, true) => Verdict::RemovableAtTarget,
        }
    }

    /// Machine token used in JSON output.
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Risk => "risk",
            Verdict::Removable => "removable",
            Verdict::RemovableAtTarget => "removable_at_target",
            Verdict::Info => "info",
        }
    }

    /// Fixed-width label used in the human report.
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Risk => "RISK     ",
            Verdict::Removable => "REMOVABLE",
            Verdict::RemovableAtTarget => "AT TARGET",
            Verdict::Info => "INFO     ",
        }
    }

    /// The section heading this verdict is filed under.
    pub fn heading(self) -> &'static str {
        match self {
            Verdict::Risk => "RISK -- used, but not safe for your target yet",
            Verdict::Removable => {
                "REMOVABLE -- Baseline Widely available, so the safety net can go"
            }
            Verdict::RemovableAtTarget => {
                "REMOVABLE AT YOUR TARGET -- redundant for this target, not yet Widely available"
            }
            Verdict::Info => "INFO -- pointers only, nothing to delete yet",
        }
    }

    /// ANSI SGR parameters for the human report.
    pub fn color(self) -> &'static str {
        match self {
            Verdict::Risk => "31",
            Verdict::Removable => "32",
            Verdict::RemovableAtTarget => "33",
            Verdict::Info => "2",
        }
    }
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// When a run should exit non-zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FailPolicy {
    /// `--fail-on risk` (the default): fail when anything is unsafe for the
    /// target. Removable findings are cleanup, not breakage, so they pass.
    #[default]
    Risk,
    /// `--strict`: fail on any finding at all, in either direction.
    Any,
    /// `--fail-on none`: always exit 0 unless something actually went wrong.
    None,
}

impl FailPolicy {
    /// Parse a `--fail-on` value. `--strict` is a separate flag.
    pub fn parse(s: &str) -> Result<FailPolicy, String> {
        match s {
            "risk" => Ok(FailPolicy::Risk),
            "none" => Ok(FailPolicy::None),
            _ => Err(format!(
                "unknown --fail-on value '{s}'; expected risk or none"
            )),
        }
    }
}

/// One matched line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineFinding {
    /// 1-based.
    pub line: usize,
    /// 1-based character column of the leftmost match.
    pub column: usize,
    /// The trimmed, capped source line.
    pub snippet: String,
    /// Which detectors fired, left to right.
    pub detectors: Vec<MatchedDetector>,
}

/// Every matched line in one file, for one feature and direction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFindings {
    /// Repository-relative path with forward slashes.
    pub path: String,
    /// Ascending by line.
    pub lines: Vec<LineFinding>,
}

/// Everything found for one feature, in one direction.
#[derive(Debug, Clone, PartialEq)]
pub struct FeatureFindings {
    pub feature: Feature,
    pub direction: Direction,
    pub verdict: Verdict,
    /// Whether the feature has reached the run's target.
    pub satisfies_target: bool,
    /// Ascending by path.
    pub files: Vec<FileFindings>,
    /// Total matched lines across every file.
    pub total: usize,
}

impl FeatureFindings {
    /// The advice to print for this group.
    pub fn action(&self) -> &str {
        self.feature.action(self.direction)
    }
}

/// How many files the walk looked at and why it skipped the rest.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanStats {
    /// Files whose contents were matched against the catalog.
    pub scanned: usize,
    /// Skipped because the extension is not in the default source set.
    pub skipped_extension: usize,
    /// Skipped because they were larger than the size cap.
    pub skipped_large: usize,
    /// Skipped because they were binary or not valid UTF-8.
    pub skipped_binary: usize,
    /// Skipped because they could not be read.
    pub skipped_unreadable: usize,
}

impl ScanStats {
    pub fn skipped_total(&self) -> usize {
        self.skipped_extension + self.skipped_large + self.skipped_binary + self.skipped_unreadable
    }
}

/// The finished report.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// Display name of the scanned directory.
    pub root_name: String,
    /// The Baseline level the run was judged against.
    pub target: Target,
    /// Set when `--target browserslist` resolved the level from a config file.
    pub target_source: Option<BrowserslistSource>,
    /// Provenance of the embedded snapshot.
    pub dataset: Provenance,
    /// How many features were actually searched for.
    pub features_searched: usize,
    /// Groups with at least one finding, in report order.
    pub groups: Vec<FeatureFindings>,
    pub stats: ScanStats,
}

/// One (feature, direction) pair while grouping, holding its matched lines per
/// file. Local to [`Report::build`]; the sorted result is [`FeatureFindings`].
struct Bucket {
    /// Index into the catalog's features.
    index: usize,
    direction: Direction,
    /// Repository-relative path to the lines matched in it.
    per_path: Vec<(String, Vec<LineFinding>)>,
}

impl Report {
    /// Group raw hits into the report.
    ///
    /// `hits` is `(repository-relative path, hits in that file)`. Ordering of
    /// the input does not matter: everything is sorted here, so callers cannot
    /// produce unstable output by walking the tree in a different order.
    pub fn build(
        root_name: String,
        target: Target,
        target_source: Option<BrowserslistSource>,
        catalog: &Catalog,
        hits: Vec<FileHits>,
        stats: ScanStats,
    ) -> Report {
        let mut buckets: Vec<Bucket> = Vec::new();

        for (path, file_hits) in hits {
            for hit in file_hits {
                if hit.feature_index >= catalog.features.len() {
                    continue;
                }
                let finding = LineFinding {
                    line: hit.line,
                    column: hit.column(),
                    snippet: hit.snippet,
                    detectors: hit.detectors,
                };
                let bucket = match buckets
                    .iter_mut()
                    .position(|b| b.index == hit.feature_index && b.direction == hit.direction)
                {
                    Some(at) => &mut buckets[at],
                    None => {
                        buckets.push(Bucket {
                            index: hit.feature_index,
                            direction: hit.direction,
                            per_path: Vec::new(),
                        });
                        buckets.last_mut().expect("just pushed")
                    }
                };
                match bucket.per_path.iter_mut().find(|(p, _)| *p == path) {
                    Some((_, lines)) => lines.push(finding),
                    None => bucket.per_path.push((path.clone(), vec![finding])),
                }
            }
        }

        let mut groups = Vec::with_capacity(buckets.len());
        for Bucket {
            index,
            direction,
            mut per_path,
        } in buckets
        {
            let feature = catalog.features[index].clone();
            let satisfies_target = target.satisfied_by(&feature);
            per_path.sort_by(|a, b| a.0.cmp(&b.0));
            let mut total = 0;
            let files: Vec<FileFindings> = per_path
                .drain(..)
                .map(|(path, mut lines)| {
                    lines.sort_by_key(|l| (l.line, l.column));
                    total += lines.len();
                    FileFindings { path, lines }
                })
                .collect();
            groups.push(FeatureFindings {
                verdict: Verdict::of(direction, satisfies_target, feature.baseline),
                feature,
                direction,
                satisfies_target,
                files,
                total,
            });
        }

        // Report order: risk before cleanup; within risk the least available
        // feature first, because that is the one least likely to be fixable by
        // waiting; then by id so two runs never disagree.
        groups.sort_by(|a, b| {
            a.verdict
                .cmp(&b.verdict)
                .then_with(|| a.feature.baseline.cmp(&b.feature.baseline))
                .then_with(|| a.feature.id.cmp(&b.feature.id))
        });

        Report {
            root_name,
            target,
            target_source,
            dataset: catalog.dataset.clone(),
            features_searched: catalog.features.len(),
            groups,
            stats,
        }
    }

    /// Total matched lines across every group.
    pub fn total_findings(&self) -> usize {
        self.groups.iter().map(|g| g.total).sum()
    }

    /// Matched lines carrying `verdict`.
    pub fn count_of(&self, verdict: Verdict) -> usize {
        self.groups
            .iter()
            .filter(|g| g.verdict == verdict)
            .map(|g| g.total)
            .sum()
    }

    /// Number of distinct files with at least one finding.
    pub fn files_with_findings(&self) -> usize {
        let mut paths: Vec<&str> = self
            .groups
            .iter()
            .flat_map(|g| g.files.iter().map(|f| f.path.as_str()))
            .collect();
        paths.sort_unstable();
        paths.dedup();
        paths.len()
    }

    /// Number of distinct features with at least one finding.
    pub fn features_matched(&self) -> usize {
        let mut ids: Vec<&str> = self.groups.iter().map(|g| g.feature.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        ids.len()
    }

    /// Groups filed under `verdict`, in report order.
    pub fn groups_of(&self, verdict: Verdict) -> impl Iterator<Item = &FeatureFindings> {
        self.groups.iter().filter(move |g| g.verdict == verdict)
    }

    /// True when `policy` considers this report a failure.
    pub fn breached(&self, policy: FailPolicy) -> bool {
        match policy {
            FailPolicy::None => false,
            FailPolicy::Any => !self.groups.is_empty(),
            FailPolicy::Risk => self.groups.iter().any(|g| g.verdict == Verdict::Risk),
        }
    }

    /// Process exit code: 0 clean, 1 threshold breached. Usage and I/O errors
    /// exit 2 from `main`, never from here.
    pub fn exit_code(&self, policy: FailPolicy) -> i32 {
        i32::from(self.breached(policy))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::MatchKind;
    use crate::matcher::Hit;

    fn index_of(catalog: &Catalog, id: &str) -> usize {
        catalog
            .features
            .iter()
            .position(|f| f.id == id)
            .unwrap_or_else(|| panic!("{id} must be in the snapshot"))
    }

    fn hit(catalog: &Catalog, id: &str, direction: Direction, line: usize) -> Hit {
        Hit {
            feature_index: index_of(catalog, id),
            direction,
            line,
            snippet: format!("line {line}"),
            detectors: vec![MatchedDetector {
                text: "x".into(),
                kind: MatchKind::Property,
                what: "w".into(),
                action: None,
                column: 3,
            }],
        }
    }

    fn report(target: Target, hits: Vec<FileHits>) -> Report {
        Report::build(
            "repo".into(),
            target,
            None,
            &Catalog::builtin(),
            hits,
            ScanStats::default(),
        )
    }

    #[test]
    fn verdicts_separate_widely_available_from_merely_target_satisfying() {
        assert_eq!(
            Verdict::of(Direction::Risk, false, Baseline::Limited),
            Verdict::Risk
        );
        assert_eq!(
            Verdict::of(Direction::Removable, true, Baseline::Widely),
            Verdict::Removable
        );
        assert_eq!(
            Verdict::of(Direction::Removable, true, Baseline::Newly),
            Verdict::RemovableAtTarget
        );
        assert_eq!(
            Verdict::of(Direction::Removable, false, Baseline::Limited),
            Verdict::Info
        );
        for v in [
            Verdict::Risk,
            Verdict::Removable,
            Verdict::RemovableAtTarget,
            Verdict::Info,
        ] {
            assert_eq!(v.label().len(), 9, "columns must line up: {v}");
            assert!(!v.heading().is_empty());
        }
    }

    #[test]
    fn a_prefix_is_only_unqualified_when_the_feature_is_widely_available() {
        let cat = Catalog::builtin();
        // backdrop-filter is Newly available, not Widely.
        let hits = vec![(
            "a.css".to_string(),
            vec![hit(&cat, "backdrop-filter", Direction::Removable, 1)],
        )];
        assert_eq!(
            report(Target::Newly, hits.clone()).groups[0].verdict,
            Verdict::RemovableAtTarget
        );
        // box-shadow is Widely available.
        let hits = vec![(
            "a.css".to_string(),
            vec![hit(&cat, "box-shadow", Direction::Removable, 1)],
        )];
        assert_eq!(
            report(Target::Widely, hits).groups[0].verdict,
            Verdict::Removable
        );
    }

    #[test]
    fn a_removable_finding_on_an_unreached_feature_is_only_information() {
        let cat = Catalog::builtin();
        let hits = vec![(
            "a.js".to_string(),
            vec![hit(&cat, "anchor-positioning", Direction::Removable, 1)],
        )];
        let r = report(Target::Widely, hits);
        assert_eq!(r.groups[0].verdict, Verdict::Info);
        assert!(!r.groups[0].satisfies_target);
        assert!(r.groups[0].action().contains("Do not remove"));
    }

    #[test]
    fn findings_group_by_feature_and_direction_then_file_then_line() {
        let cat = Catalog::builtin();
        let r = report(
            Target::Widely,
            vec![
                (
                    "z.css".into(),
                    vec![
                        hit(&cat, "anchor-positioning", Direction::Risk, 9),
                        hit(&cat, "anchor-positioning", Direction::Risk, 2),
                    ],
                ),
                (
                    "a.css".into(),
                    vec![
                        hit(&cat, "anchor-positioning", Direction::Risk, 7),
                        hit(&cat, "anchor-positioning", Direction::Removable, 4),
                    ],
                ),
            ],
        );
        assert_eq!(r.groups.len(), 2);
        assert_eq!(r.groups[0].verdict, Verdict::Risk);
        assert_eq!(r.groups[1].verdict, Verdict::Info);

        let paths: Vec<&str> = r.groups[0].files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["a.css", "z.css"]);
        let lines: Vec<usize> = r.groups[0].files[1].lines.iter().map(|l| l.line).collect();
        assert_eq!(lines, [2, 9]);

        assert_eq!(r.total_findings(), 4);
        assert_eq!(r.files_with_findings(), 2);
        assert_eq!(r.features_matched(), 1);
    }

    #[test]
    fn risk_groups_sort_by_how_far_the_feature_is_from_baseline() {
        let cat = Catalog::builtin();
        let r = report(
            Target::Widely,
            vec![(
                "a.css".into(),
                vec![
                    // has is Widely available; a year target is what would make
                    // it a risk, but the ordering key is the baseline itself.
                    hit(&cat, "has", Direction::Risk, 1),
                    hit(&cat, "anchor-positioning", Direction::Risk, 1),
                    hit(&cat, "backdrop-filter", Direction::Risk, 1),
                ],
            )],
        );
        let ids: Vec<&str> = r.groups.iter().map(|g| g.feature.id.as_str()).collect();
        assert_eq!(ids, ["anchor-positioning", "backdrop-filter", "has"]);
    }

    #[test]
    fn the_target_decides_which_side_of_the_report_a_feature_lands_on() {
        let cat = Catalog::builtin();
        let hits = vec![(
            "a.css".to_string(),
            vec![hit(&cat, "has", Direction::Removable, 1)],
        )];
        // :has() is Widely available, so its polyfill is removable.
        assert_eq!(
            report(Target::Widely, hits.clone()).groups[0].verdict,
            Verdict::Removable
        );
        // Target a year before it was even Newly available and the polyfill is
        // load-bearing again, so the finding drops to information.
        assert_eq!(
            report(Target::Year(2020), hits).groups[0].verdict,
            Verdict::Info
        );
    }

    #[test]
    fn a_clean_scan_reports_nothing_and_still_knows_the_catalog_size() {
        let r = report(Target::Widely, vec![("a.css".into(), vec![])]);
        assert!(r.groups.is_empty());
        assert_eq!(r.total_findings(), 0);
        assert_eq!(r.features_matched(), 0);
        assert!(r.features_searched >= 40);
        assert_eq!(r.dataset.package, "web-features");
    }

    #[test]
    fn the_default_policy_fails_on_risk_and_forgives_cleanup() {
        let cat = Catalog::builtin();
        let risky = report(
            Target::Widely,
            vec![("a.css".into(), vec![hit(&cat, "has", Direction::Risk, 1)])],
        );
        assert!(risky.breached(FailPolicy::Risk));
        assert_eq!(risky.exit_code(FailPolicy::Risk), 1);
        assert_eq!(risky.exit_code(FailPolicy::None), 0);

        let cleanup = report(
            Target::Widely,
            vec![(
                "a.css".into(),
                vec![hit(&cat, "box-shadow", Direction::Removable, 1)],
            )],
        );
        assert!(!cleanup.breached(FailPolicy::Risk));
        assert!(cleanup.breached(FailPolicy::Any));
        assert_eq!(cleanup.exit_code(FailPolicy::Risk), 0);
        assert_eq!(cleanup.exit_code(FailPolicy::Any), 1);
    }

    #[test]
    fn strict_on_a_clean_report_still_exits_zero() {
        let r = report(Target::Widely, vec![]);
        assert_eq!(r.exit_code(FailPolicy::Any), 0);
        assert_eq!(r.exit_code(FailPolicy::Risk), 0);
    }

    #[test]
    fn fail_policies_parse_their_documented_values() {
        assert_eq!(FailPolicy::parse("risk").unwrap(), FailPolicy::Risk);
        assert_eq!(FailPolicy::parse("none").unwrap(), FailPolicy::None);
        assert_eq!(FailPolicy::default(), FailPolicy::Risk);
        let err = FailPolicy::parse("all").unwrap_err();
        assert!(err.contains("risk or none"), "{err}");
    }

    #[test]
    fn counts_split_by_verdict() {
        let cat = Catalog::builtin();
        let r = report(
            Target::Widely,
            vec![(
                "a.css".into(),
                vec![
                    hit(&cat, "has", Direction::Risk, 1),
                    hit(&cat, "has", Direction::Risk, 2),
                    hit(&cat, "box-shadow", Direction::Removable, 3),
                ],
            )],
        );
        assert_eq!(r.count_of(Verdict::Risk), 2);
        assert_eq!(r.count_of(Verdict::Removable), 1);
        assert_eq!(r.count_of(Verdict::Info), 0);
        assert_eq!(r.groups_of(Verdict::Risk).count(), 1);
    }

    #[test]
    fn scan_stats_sum_their_skips() {
        let stats = ScanStats {
            scanned: 10,
            skipped_extension: 1,
            skipped_large: 2,
            skipped_binary: 3,
            skipped_unreadable: 4,
        };
        assert_eq!(stats.skipped_total(), 10);
    }
}
