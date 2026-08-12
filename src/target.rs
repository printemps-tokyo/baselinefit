//! What "safe to use" means for this project.
//!
//! Baseline has two named levels and a family of year labels, defined at
//! <https://web.dev/baseline> (read 2026-08-12):
//!
//! - **Newly available** -- "The feature is supported by all of the core
//!   browsers, and is therefore interoperable." The core browsers are Chrome
//!   (desktop and Android), Edge, Firefox (desktop and Android) and Safari
//!   (macOS and iOS).
//! - **Widely available** -- "30 months have passed since the newly
//!   interoperable date. The feature can be used by most sites without worrying
//!   about support."
//! - A year label collects the features that became Newly available during that
//!   year: "All items that become part of Baseline Newly available in 2025 can
//!   be referred to as Baseline 2025."
//!
//! Those are the same three shapes Browserslist exposes as `baseline widely
//! available`, `baseline newly available` and `baseline YYYY`
//! (<https://web.dev/articles/use-baseline-with-browserslist>, read
//! 2026-08-12), which is why `--target` accepts exactly them and no more.

use crate::catalog::{Baseline, Feature};

/// The lowest Baseline level a feature must have reached to count as safe.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Target {
    /// Baseline Widely available: interoperable for at least 30 months. The
    /// default, because it is the level that needs no further justification.
    #[default]
    Widely,
    /// Baseline Newly available: interoperable in the core browser set.
    Newly,
    /// Baseline `year`: everything that became Newly available in that calendar
    /// year or earlier.
    Year(i32),
}

/// The earliest year Browserslist's `baseline YYYY` accepts. The web.dev
/// article states "You can specify years from `baseline 2015` to the current
/// year."
pub const FIRST_BASELINE_YEAR: i32 = 2015;

/// An upper bound on the year, so a typo like `--target 20226` is rejected
/// rather than silently behaving like `newly`.
pub const LAST_BASELINE_YEAR: i32 = 2100;

impl Target {
    /// Parse a `--target` value. `browserslist` is handled by the caller,
    /// because it needs the filesystem.
    pub fn parse(s: &str) -> Result<Target, String> {
        match s {
            "widely" => Ok(Target::Widely),
            "newly" => Ok(Target::Newly),
            _ => {
                let year: i32 = s.parse().map_err(|_| {
                    format!(
                        "unknown target '{s}'; expected widely, newly, a year like 2026, \
                         or browserslist"
                    )
                })?;
                Target::year(year)
            }
        }
    }

    /// Build a year target, range-checked.
    pub fn year(year: i32) -> Result<Target, String> {
        if !(FIRST_BASELINE_YEAR..=LAST_BASELINE_YEAR).contains(&year) {
            return Err(format!(
                "baseline year {year} is out of range; Baseline year labels start at \
                 {FIRST_BASELINE_YEAR}"
            ));
        }
        Ok(Target::Year(year))
    }

    /// How the target is written on the command line and in JSON.
    pub fn as_str(self) -> String {
        match self {
            Target::Widely => "widely".to_string(),
            Target::Newly => "newly".to_string(),
            Target::Year(y) => y.to_string(),
        }
    }

    /// The Browserslist query this target corresponds to, for the report
    /// footer. Lets a reader carry the same intent into their bundler config.
    pub fn browserslist_query(self) -> String {
        match self {
            Target::Widely => "baseline widely available".to_string(),
            Target::Newly => "baseline newly available".to_string(),
            Target::Year(y) => format!("baseline {y}"),
        }
    }

    /// A one-line description used in the human report header.
    pub fn describe(self) -> String {
        match self {
            Target::Widely => {
                "Baseline Widely available (interoperable for 30+ months)".to_string()
            }
            Target::Newly => "Baseline Newly available (interoperable now)".to_string(),
            Target::Year(y) => format!("Baseline {y} (Newly available in {y} or earlier)"),
        }
    }

    /// True when `feature` has reached this target and can be relied on.
    ///
    /// This is the single pivot the whole tool turns on. A feature that
    /// satisfies the target is safe to use (so no risk finding) and safe to
    /// stop guarding (so its prefixes, `@supports` wrappers and polyfills
    /// become removable). A feature that does not satisfy it is the reverse.
    pub fn satisfied_by(self, feature: &Feature) -> bool {
        match self {
            Target::Widely => feature.baseline == Baseline::Widely,
            Target::Newly => matches!(feature.baseline, Baseline::Widely | Baseline::Newly),
            // Baseline YYYY is defined by when a feature became *newly*
            // available, so the low date is the one that matters here; the high
            // date plays no part.
            Target::Year(y) => feature.baseline_low_date.is_some_and(|d| d.year() <= y),
        }
    }
}

// ---------------------------------------------------------------------------
// browserslist
// ---------------------------------------------------------------------------

/// Where a resolved Browserslist target was read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserslistSource {
    /// Repository-relative path of the config that supplied the query.
    pub path: String,
    /// The query as written.
    pub query: String,
}

/// Turn the contents of a Browserslist config into a [`Target`].
///
/// baselinefit does not embed the Browserslist engine, and mapping an arbitrary
/// query such as `> 0.5%, last 2 versions, not dead` onto a Baseline level is
/// not something that can be done honestly without it: the answer depends on
/// caniuse usage data that changes weekly. So only the three Baseline query
/// forms the web.dev article documents are accepted, and anything else is
/// refused with an explanation rather than guessed at.
///
/// `with downstream` is accepted and ignored: it widens the resolved browser
/// list to Chromium forks, which does not change which Baseline level is being
/// asked for.
pub fn parse_browserslist(contents: &str) -> Result<(Target, String), String> {
    let queries = split_queries(contents)?;
    match queries.len() {
        0 => Err("the browserslist config is empty".to_string()),
        1 => parse_query(&queries[0]).map(|t| (t, queries[0].clone())),
        _ => Err(format!(
            "the browserslist config combines {} queries ({}); baselinefit can only map a \
             config that is a single Baseline query. Pass --target explicitly.",
            queries.len(),
            queries.join(" | ")
        )),
    }
}

/// Split a `.browserslistrc` body or a joined `package.json` array into
/// individual queries, dropping comments and blank lines.
fn split_queries(contents: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for raw in contents.lines() {
        let line = match raw.split_once('#') {
            Some((before, _)) => before,
            None => raw,
        }
        .trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            return Err(format!(
                "the browserslist config uses environment sections ({line}); baselinefit cannot \
                 tell which one applies. Pass --target explicitly."
            ));
        }
        for part in line.split(',') {
            let part = part.trim();
            if !part.is_empty() {
                out.push(part.to_string());
            }
        }
    }
    Ok(out)
}

/// Map one query string onto a target, or explain why it cannot be mapped.
fn parse_query(query: &str) -> Result<Target, String> {
    // Collapse the run-together whitespace first, then peel off the optional
    // `with downstream` suffix, so both orderings of sloppiness are handled.
    let collapsed = query
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let normalized = collapsed
        .strip_suffix(" with downstream")
        .unwrap_or(&collapsed)
        .to_string();

    let unsupported = || {
        format!(
            "unsupported browserslist query '{query}'; baselinefit understands only \
             'baseline widely available', 'baseline newly available' and 'baseline YYYY' \
             (optionally with 'with downstream'). Pass --target explicitly."
        )
    };

    let Some(rest) = normalized.strip_prefix("baseline ") else {
        return Err(unsupported());
    };
    match rest {
        "widely available" => Ok(Target::Widely),
        "newly available" => Ok(Target::Newly),
        _ => {
            let year: i32 = rest.parse().map_err(|_| unsupported())?;
            Target::year(year)
        }
    }
}

/// Pull the `browserslist` value out of a `package.json` document and join it
/// into the newline-separated form [`parse_browserslist`] expects.
pub fn browserslist_from_package_json(text: &str) -> Result<Option<String>, String> {
    let doc = crate::json::parse(text).map_err(|e| format!("package.json: {e}"))?;
    let Some(value) = doc.get("browserslist") else {
        return Ok(None);
    };
    match value {
        crate::json::Value::String(s) => Ok(Some(s.clone())),
        crate::json::Value::Array(items) => {
            let mut lines = Vec::with_capacity(items.len());
            for item in items {
                let Some(s) = item.as_str() else {
                    return Err(
                        "package.json: every \"browserslist\" entry must be a string".to_string(),
                    );
                };
                lines.push(s.to_string());
            }
            Ok(Some(lines.join("\n")))
        }
        crate::json::Value::Object(_) => Err(
            "package.json: \"browserslist\" uses per-environment sections; baselinefit cannot \
             tell which one applies. Pass --target explicitly."
                .to_string(),
        ),
        other => Err(format!(
            "package.json: \"browserslist\" must be a string or an array, found a {}",
            other.type_name()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;

    fn feature(id: &str) -> Feature {
        Catalog::builtin()
            .features
            .iter()
            .find(|f| f.id == id)
            .unwrap_or_else(|| panic!("{id} must be in the snapshot"))
            .clone()
    }

    #[test]
    fn target_parses_the_three_documented_forms() {
        assert_eq!(Target::parse("widely").unwrap(), Target::Widely);
        assert_eq!(Target::parse("newly").unwrap(), Target::Newly);
        assert_eq!(Target::parse("2026").unwrap(), Target::Year(2026));
        assert_eq!(Target::parse("2015").unwrap(), Target::Year(2015));
    }

    #[test]
    fn target_rejects_nonsense_and_out_of_range_years() {
        for bad in ["Widely", "wide", "", "2026-01", "20226", "2014", "-1"] {
            assert!(Target::parse(bad).is_err(), "{bad} should not parse");
        }
        assert!(Target::parse("2014").unwrap_err().contains("2015"));
    }

    #[test]
    fn target_round_trips_through_its_string_form() {
        for t in [Target::Widely, Target::Newly, Target::Year(2024)] {
            assert_eq!(Target::parse(&t.as_str()).unwrap(), t);
        }
    }

    #[test]
    fn widely_accepts_only_high_baseline() {
        // container-queries reached Widely available on 2025-08-14.
        assert!(Target::Widely.satisfied_by(&feature("container-queries")));
        // backdrop-filter is Newly available only (2024-09-16).
        assert!(!Target::Widely.satisfied_by(&feature("backdrop-filter")));
        // anchor-positioning is not Baseline at all.
        assert!(!Target::Widely.satisfied_by(&feature("anchor-positioning")));
    }

    #[test]
    fn newly_accepts_high_and_low_but_not_false() {
        assert!(Target::Newly.satisfied_by(&feature("container-queries")));
        assert!(Target::Newly.satisfied_by(&feature("backdrop-filter")));
        assert!(!Target::Newly.satisfied_by(&feature("anchor-positioning")));
    }

    #[test]
    fn a_year_target_keys_on_the_newly_available_date() {
        // :has() became Newly available on 2023-12-19 and Widely on 2026-06-19.
        let has = feature("has");
        assert!(!Target::Year(2022).satisfied_by(&has));
        assert!(
            Target::Year(2023).satisfied_by(&has),
            "inclusive at the year"
        );
        assert!(Target::Year(2024).satisfied_by(&has));
        // A feature that never became Newly available is never in a year set.
        assert!(!Target::Year(2100).satisfied_by(&feature("anchor-positioning")));
    }

    #[test]
    fn every_target_form_has_a_browserslist_query() {
        assert_eq!(
            Target::Widely.browserslist_query(),
            "baseline widely available"
        );
        assert_eq!(
            Target::Newly.browserslist_query(),
            "baseline newly available"
        );
        assert_eq!(Target::Year(2024).browserslist_query(), "baseline 2024");
        // The queries we print are the ones we can read back.
        for t in [Target::Widely, Target::Newly, Target::Year(2024)] {
            assert_eq!(parse_query(&t.browserslist_query()).unwrap(), t);
        }
    }

    #[test]
    fn browserslist_reads_the_baseline_queries() {
        for (body, expected) in [
            ("baseline widely available", Target::Widely),
            ("baseline newly available", Target::Newly),
            ("baseline 2024", Target::Year(2024)),
            ("Baseline Widely Available", Target::Widely),
            ("baseline widely available with downstream", Target::Widely),
            ("baseline 2024 with downstream", Target::Year(2024)),
            ("  baseline   newly   available  ", Target::Newly),
            ("# a comment\nbaseline 2023\n", Target::Year(2023)),
        ] {
            let (target, _) = parse_browserslist(body).unwrap_or_else(|e| panic!("{body}: {e}"));
            assert_eq!(target, expected, "{body}");
        }
    }

    #[test]
    fn browserslist_refuses_to_guess_at_anything_else() {
        for body in [
            "> 0.5%",
            "last 2 versions",
            "defaults",
            "not dead",
            "chrome >= 100",
            "baseline 2014",
            "baseline soon",
        ] {
            let err = parse_browserslist(body).unwrap_err();
            assert!(
                err.contains("--target") || err.contains("out of range"),
                "{body} -> {err}"
            );
        }
    }

    #[test]
    fn browserslist_refuses_combined_queries_and_env_sections() {
        let err = parse_browserslist("baseline widely available, not dead").unwrap_err();
        assert!(err.contains("combines 2 queries"), "{err}");
        let err = parse_browserslist("last 2 versions\nnot dead").unwrap_err();
        assert!(err.contains("combines 2 queries"), "{err}");
        let err = parse_browserslist("[production]\nbaseline 2024").unwrap_err();
        assert!(err.contains("environment sections"), "{err}");
        assert!(parse_browserslist("\n# only comments\n").is_err());
    }

    #[test]
    fn package_json_browserslist_accepts_a_string_or_an_array() {
        assert_eq!(
            browserslist_from_package_json(r#"{"browserslist": "baseline 2024"}"#).unwrap(),
            Some("baseline 2024".to_string())
        );
        assert_eq!(
            browserslist_from_package_json(r#"{"browserslist": ["baseline widely available"]}"#)
                .unwrap(),
            Some("baseline widely available".to_string())
        );
        assert_eq!(
            browserslist_from_package_json(r#"{"name": "x"}"#).unwrap(),
            None
        );
        let err = browserslist_from_package_json(
            r#"{"browserslist": {"production": ["baseline 2024"]}}"#,
        )
        .unwrap_err();
        assert!(err.contains("per-environment"), "{err}");
        assert!(browserslist_from_package_json(r#"{"browserslist": 3}"#).is_err());
        assert!(browserslist_from_package_json("not json").is_err());
    }
}
