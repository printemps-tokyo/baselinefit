//! The feature snapshot: what baselinefit knows about Baseline, and what text
//! to look for in a repository.
//!
//! `src/features.json` is embedded at build time and is the only place Baseline
//! facts live. It is generated, never hand-edited: `tools/fetch-data.py`
//! downloads the published `web-features` npm package and copies each feature's
//! `name`, `description`, `spec`, `status.baseline`, `status.baseline_low_date`
//! and `status.baseline_high_date` verbatim, for exactly the ids listed in
//! `tools/detectors.json`. An id that is not in the dataset fails the
//! generator, so no status or date in this tool can have been invented.
//!
//! The generator also stamps the dataset version, the tarball URL, its SHA-256
//! and the retrieval date into the file. Those are printed under every report
//! and by `--list`, because a snapshot with no date on it is a rumour.

use crate::date::Date;
use crate::json::{self, Value};

/// The snapshot compiled into this binary.
pub const BUILTIN_JSON: &str = include_str!("features.json");

/// The only schema version this build understands.
pub const SCHEMA_VERSION: u32 = 1;

/// A feature's Baseline status, mirroring `status.baseline` in web-features.
///
/// The dataset spells these `"high"`, `"low"` and `false`; baselinefit uses the
/// names web.dev uses in prose so the report reads the way the documentation
/// does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Baseline {
    /// `status.baseline: false`. Not interoperable across the core browsers.
    Limited,
    /// `status.baseline: "low"`. Newly available.
    Newly,
    /// `status.baseline: "high"`. Widely available, i.e. Newly available at
    /// least 30 months ago.
    Widely,
}

impl Baseline {
    /// The dataset's own spelling, used in JSON output.
    pub fn as_dataset_str(self) -> &'static str {
        match self {
            Baseline::Limited => "false",
            Baseline::Newly => "low",
            Baseline::Widely => "high",
        }
    }

    /// The prose label web.dev uses.
    pub fn label(self) -> &'static str {
        match self {
            Baseline::Limited => "Limited availability",
            Baseline::Newly => "Newly available",
            Baseline::Widely => "Widely available",
        }
    }

    /// A short label for fixed-width columns.
    pub fn short(self) -> &'static str {
        match self {
            Baseline::Limited => "limited",
            Baseline::Newly => "newly  ",
            Baseline::Widely => "widely ",
        }
    }

    fn parse(s: &str) -> Option<Baseline> {
        match s {
            "false" => Some(Baseline::Limited),
            "low" => Some(Baseline::Newly),
            "high" => Some(Baseline::Widely),
            _ => None,
        }
    }
}

/// Which of the two questions a detector answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Direction {
    /// "Is this safe for my target yet?" -- a use of the feature itself.
    Risk,
    /// "Can I delete this now?" -- a prefix, guard or polyfill that exists only
    /// because the feature was once unavailable.
    Removable,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Risk => "risk",
            Direction::Removable => "removable",
        }
    }

    pub fn parse(s: &str) -> Option<Direction> {
        match s {
            "risk" => Some(Direction::Risk),
            "removable" => Some(Direction::Removable),
            _ => None,
        }
    }
}

/// When a detector produces a finding, relative to the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    /// Report only when the feature has reached the target. Prefixes, guards
    /// and polyfills: the safety net is dead weight once the feature is there.
    Satisfied,
    /// Report only when the feature has *not* reached the target. Uses of the
    /// feature itself.
    Unsatisfied,
    /// Report either way. Reserved for pointers at a future replacement, which
    /// are informational and must not read as "delete this".
    Always,
}

impl When {
    pub fn as_str(self) -> &'static str {
        match self {
            When::Satisfied => "satisfied",
            When::Unsatisfied => "unsatisfied",
            When::Always => "always",
        }
    }

    fn parse(s: &str) -> Option<When> {
        match s {
            "satisfied" => Some(When::Satisfied),
            "unsatisfied" => Some(When::Unsatisfied),
            "always" => Some(When::Always),
            _ => None,
        }
    }

    /// True when a target verdict of `satisfied` should produce a finding.
    pub fn fires(self, satisfied: bool) -> bool {
        match self {
            When::Satisfied => satisfied,
            When::Unsatisfied => !satisfied,
            When::Always => true,
        }
    }
}

/// How a detector's text is matched against a line. See `matcher.rs` for the
/// precise rule each one applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    /// `@container`, `@-webkit-keyframes`.
    AtRule,
    /// A CSS property name in a declaration: the exact ident, then `:`.
    Property,
    /// A CSS property name *prefix*: the ident starts with the text, runs to
    /// the end of the ident, then `:`. `-webkit-mask` covers `-webkit-mask-image`.
    PropertyStem,
    /// `prop:value` -- a property with one specific value, whitespace-tolerant.
    Declaration,
    /// A selector fragment: `:has(`, `::view-transition`.
    Selector,
    /// A CSS function call: text ends with `(`.
    Function,
    /// A CSS unit that must follow a number: `1lh`, `100dvh`.
    Unit,
    /// A JavaScript identifier or dotted member path: `SharedWorker`,
    /// `navigator.clipboard`.
    Ident,
    /// A package, module or file name: `resize-observer-polyfill`.
    Package,
    /// A condition inside `@supports`: text is `prop` or `prop:value`.
    Supports,
    /// A selector inside `@supports selector(...)`.
    SupportsSelector,
}

impl MatchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MatchKind::AtRule => "at-rule",
            MatchKind::Property => "property",
            MatchKind::PropertyStem => "property-stem",
            MatchKind::Declaration => "declaration",
            MatchKind::Selector => "selector",
            MatchKind::Function => "function",
            MatchKind::Unit => "unit",
            MatchKind::Ident => "ident",
            MatchKind::Package => "package",
            MatchKind::Supports => "supports",
            MatchKind::SupportsSelector => "supports-selector",
        }
    }

    pub fn parse(s: &str) -> Option<MatchKind> {
        match s {
            "at-rule" => Some(MatchKind::AtRule),
            "property" => Some(MatchKind::Property),
            "property-stem" => Some(MatchKind::PropertyStem),
            "declaration" => Some(MatchKind::Declaration),
            "selector" => Some(MatchKind::Selector),
            "function" => Some(MatchKind::Function),
            "unit" => Some(MatchKind::Unit),
            "ident" => Some(MatchKind::Ident),
            "package" => Some(MatchKind::Package),
            "supports" => Some(MatchKind::Supports),
            "supports-selector" => Some(MatchKind::SupportsSelector),
            _ => None,
        }
    }
}

/// One thing to look for, and what finding it produces.
#[derive(Debug, Clone, PartialEq)]
pub struct Detector {
    pub direction: Direction,
    pub when: When,
    pub kind: MatchKind,
    /// The literal text. Matching is case-sensitive.
    pub text: String,
    /// A short label for the match, e.g. "vendor prefix".
    pub what: String,
    /// Overrides the feature-level action for this detector.
    pub action: Option<String>,
    /// gitignore-syntax globs matched against the repository-relative path.
    /// Empty means "every scanned file".
    pub scope: Vec<String>,
}

/// One web-features entry, with the patterns that find it in a codebase.
#[derive(Debug, Clone, PartialEq)]
pub struct Feature {
    /// The web-features id. Also the `--only` / `--skip` value.
    pub id: String,
    /// `name`, copied from the dataset.
    pub name: String,
    /// `description`, copied from the dataset.
    pub description: String,
    /// `status.baseline`, copied from the dataset.
    pub baseline: Baseline,
    /// `status.baseline_low_date`: when the feature became Newly available.
    pub baseline_low_date: Option<Date>,
    /// `status.baseline_high_date`: when it became Widely available.
    pub baseline_high_date: Option<Date>,
    /// The first `spec` URL, when the dataset has one.
    pub spec: Option<String>,
    /// `caniuse` ids, copied from the dataset.
    pub caniuse: Vec<String>,
    /// How many `compat_features` BCD keys the dataset record carries.
    pub compat_features_count: usize,
    /// What to do about a risk finding.
    pub risk_action: Option<String>,
    /// What to do about a removable finding.
    pub removable_action: Option<String>,
    pub detectors: Vec<Detector>,
}

impl Feature {
    /// The action text for a finding in `direction`, falling back to a generic
    /// line so the report is never blank.
    pub fn action(&self, direction: Direction) -> &str {
        let specific = match direction {
            Direction::Risk => self.risk_action.as_deref(),
            Direction::Removable => self.removable_action.as_deref(),
        };
        specific.unwrap_or("See the feature's specification and decide case by case.")
    }

    /// The date this feature reached `Widely available`, or the date it became
    /// `Newly available`, whichever is the most advanced thing that has
    /// happened. Used only for display.
    pub fn milestone(&self) -> Option<(&'static str, Date)> {
        match (
            self.baseline,
            self.baseline_high_date,
            self.baseline_low_date,
        ) {
            (Baseline::Widely, Some(d), _) => Some(("widely since", d)),
            (Baseline::Newly, _, Some(d)) => Some(("newly since", d)),
            _ => None,
        }
    }
}

/// Where the embedded snapshot came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    /// The npm package the data was taken from.
    pub package: String,
    /// Its published version.
    pub version: String,
    /// The exact tarball that was downloaded.
    pub source_url: String,
    /// SHA-256 of that tarball.
    pub sha256: String,
    /// The date the generator ran.
    pub retrieved_on: Date,
    /// The script that produced the file.
    pub generator: String,
}

impl Provenance {
    /// The one-line credit printed under every report.
    pub fn credit(&self) -> String {
        format!(
            "Baseline data: {}@{}, retrieved {} by {}.",
            self.package, self.version, self.retrieved_on, self.generator
        )
    }
}

/// The parsed snapshot, in deterministic order.
#[derive(Debug, Clone, PartialEq)]
pub struct Catalog {
    pub dataset: Provenance,
    /// Sorted by id.
    pub features: Vec<Feature>,
}

impl Catalog {
    /// The snapshot compiled into this binary.
    ///
    /// # Panics
    ///
    /// Only if `src/features.json` is malformed, which the test suite rejects.
    pub fn builtin() -> Catalog {
        Catalog::from_json(BUILTIN_JSON, "<built-in>")
            .expect("the built-in snapshot is validated by the test suite")
    }

    /// Parse a snapshot document. `origin` is used in error messages.
    pub fn from_json(text: &str, origin: &str) -> Result<Catalog, String> {
        let doc = json::parse(text).map_err(|e| format!("{origin}: {e}"))?;
        let ctx = |what: &str| format!("{origin}: {what}");

        if doc.as_object().is_none() {
            return Err(ctx(&format!(
                "the document must be an object, found a {}",
                doc.type_name()
            )));
        }
        known_keys(&doc, &["schema", "dataset", "features"], origin, "")?;

        match doc.get("schema") {
            Some(Value::Number(n)) if *n == f64::from(SCHEMA_VERSION) => {}
            Some(Value::Number(n)) => {
                return Err(ctx(&format!(
                    "unsupported snapshot schema {n}; this build understands schema \
                     {SCHEMA_VERSION}"
                )))
            }
            _ => return Err(ctx(&format!("missing \"schema\": {SCHEMA_VERSION}"))),
        }

        let Some(dataset) = doc.get("dataset") else {
            return Err(ctx("missing \"dataset\" provenance block"));
        };
        known_keys(
            dataset,
            &[
                "package",
                "version",
                "source_url",
                "sha256",
                "retrieved_on",
                "generator",
                "note",
            ],
            origin,
            "dataset",
        )?;
        let dataset = Provenance {
            package: string_field(dataset, "package", origin, "dataset")?,
            version: string_field(dataset, "version", origin, "dataset")?,
            source_url: url_field(dataset, "source_url", origin, "dataset")?,
            sha256: string_field(dataset, "sha256", origin, "dataset")?,
            retrieved_on: date_field(dataset, "retrieved_on", origin, "dataset")?,
            generator: string_field(dataset, "generator", origin, "dataset")?,
        };

        let Some(items) = doc.get("features").and_then(Value::as_array) else {
            return Err(ctx("missing \"features\" array"));
        };
        if items.is_empty() {
            return Err(ctx("\"features\" must not be empty"));
        }
        let mut features = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            features.push(parse_feature(item, origin, i)?);
        }
        features.sort_by(|a, b| a.id.cmp(&b.id));
        for pair in features.windows(2) {
            if pair[0].id == pair[1].id {
                return Err(format!("{origin}: duplicate feature id '{}'", pair[0].id));
            }
        }

        Ok(Catalog { dataset, features })
    }

    /// Apply `--only` and `--skip`. An unknown id is an error rather than a
    /// silent no-op, because a typo would otherwise quietly change the scan.
    pub fn retain(&mut self, only: &[String], skip: &[String]) -> Result<(), String> {
        for id in only.iter().chain(skip) {
            if !self.features.iter().any(|f| &f.id == id) {
                return Err(format!(
                    "unknown feature id '{id}'\nrun --list to see the {} ids in the snapshot",
                    self.features.len()
                ));
            }
        }
        self.features
            .retain(|f| (only.is_empty() || only.contains(&f.id)) && !skip.contains(&f.id));
        Ok(())
    }

    /// Drop detectors that cannot produce a finding in `direction`, and then
    /// features left with nothing to look for.
    pub fn retain_direction(&mut self, keep: Option<Direction>) {
        let Some(keep) = keep else { return };
        for feature in &mut self.features {
            feature.detectors.retain(|d| d.direction == keep);
        }
        self.features.retain(|f| !f.detectors.is_empty());
    }

    /// Every id, in catalog order.
    pub fn ids(&self) -> Vec<&str> {
        self.features.iter().map(|f| f.id.as_str()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.features.is_empty()
    }

    /// Total number of detectors across every feature.
    pub fn detector_count(&self) -> usize {
        self.features.iter().map(|f| f.detectors.len()).sum()
    }
}

// ---------------------------------------------------------------------------
// Field-level parsing
// ---------------------------------------------------------------------------

fn parse_feature(value: &Value, origin: &str, index: usize) -> Result<Feature, String> {
    let label = match value.get("id").and_then(Value::as_str) {
        Some(id) => format!("feature '{id}'"),
        None => format!("feature #{}", index + 1),
    };
    if value.as_object().is_none() {
        return Err(format!(
            "{origin}: {label} must be an object, found a {}",
            value.type_name()
        ));
    }
    known_keys(
        value,
        &[
            "id",
            "name",
            "description",
            "baseline",
            "baseline_low_date",
            "baseline_high_date",
            "spec",
            "caniuse",
            "compat_features_count",
            "risk_action",
            "removable_action",
            "detectors",
        ],
        origin,
        &label,
    )?;

    let id = string_field(value, "id", origin, &label)?;
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(format!(
            "{origin}: {label}: \"id\" must be a web-features id using only [a-z0-9-]"
        ));
    }

    let baseline_str = string_field(value, "baseline", origin, &label)?;
    let Some(baseline) = Baseline::parse(&baseline_str) else {
        return Err(format!(
            "{origin}: {label}: unknown \"baseline\" '{baseline_str}' \
             (expected high, low or false, as web-features spells them)"
        ));
    };
    let baseline_low_date = optional_date_field(value, "baseline_low_date", origin, &label)?;
    let baseline_high_date = optional_date_field(value, "baseline_high_date", origin, &label)?;

    // The dataset's own invariants, re-checked here so a corrupted snapshot
    // cannot make the tool claim a feature is available when it is not.
    match baseline {
        Baseline::Widely if baseline_high_date.is_none() => {
            return Err(format!(
                "{origin}: {label}: baseline \"high\" needs a \"baseline_high_date\""
            ))
        }
        Baseline::Widely | Baseline::Newly if baseline_low_date.is_none() => {
            return Err(format!(
                "{origin}: {label}: baseline \"{baseline_str}\" needs a \"baseline_low_date\""
            ))
        }
        Baseline::Limited if baseline_low_date.is_some() || baseline_high_date.is_some() => {
            return Err(format!(
                "{origin}: {label}: baseline \"false\" must not carry Baseline dates"
            ))
        }
        _ => {}
    }
    if let (Some(low), Some(high)) = (baseline_low_date, baseline_high_date) {
        if high < low {
            return Err(format!(
                "{origin}: {label}: \"baseline_high_date\" is before \"baseline_low_date\""
            ));
        }
    }

    let Some(raw_detectors) = value.get("detectors").and_then(Value::as_array) else {
        return Err(format!("{origin}: {label}: missing \"detectors\" array"));
    };
    if raw_detectors.is_empty() {
        return Err(format!(
            "{origin}: {label}: \"detectors\" must contain at least one detector"
        ));
    }
    let mut detectors = Vec::with_capacity(raw_detectors.len());
    for (i, d) in raw_detectors.iter().enumerate() {
        detectors.push(parse_detector(d, origin, &label, i)?);
    }

    let risk_action = optional_string_field(value, "risk_action", origin, &label)?;
    let removable_action = optional_string_field(value, "removable_action", origin, &label)?;
    for (direction, action) in [
        (Direction::Risk, &risk_action),
        (Direction::Removable, &removable_action),
    ] {
        let has_detector = detectors.iter().any(|d| d.direction == direction);
        if has_detector && action.is_none() && !detectors.iter().all(|d| d.action.is_some()) {
            return Err(format!(
                "{origin}: {label}: has {} detectors but no \"{}_action\"",
                direction.as_str(),
                direction.as_str()
            ));
        }
    }

    Ok(Feature {
        id,
        name: string_field(value, "name", origin, &label)?,
        description: string_field(value, "description", origin, &label)?,
        baseline,
        baseline_low_date,
        baseline_high_date,
        spec: optional_url_field(value, "spec", origin, &label)?,
        caniuse: string_array_field(value, "caniuse", origin, &label)?,
        compat_features_count: count_field(value, "compat_features_count", origin, &label)?,
        risk_action,
        removable_action,
        detectors,
    })
}

fn parse_detector(
    value: &Value,
    origin: &str,
    feature: &str,
    index: usize,
) -> Result<Detector, String> {
    let label = format!("{feature}, detector #{}", index + 1);
    if value.as_object().is_none() {
        return Err(format!(
            "{origin}: {label} must be an object, found a {}",
            value.type_name()
        ));
    }
    known_keys(
        value,
        &[
            "direction",
            "when",
            "kind",
            "text",
            "what",
            "action",
            "scope",
        ],
        origin,
        &label,
    )?;

    let direction_str = string_field(value, "direction", origin, &label)?;
    let Some(direction) = Direction::parse(&direction_str) else {
        return Err(format!(
            "{origin}: {label}: unknown \"direction\" '{direction_str}' \
             (expected risk or removable)"
        ));
    };
    let when_str = string_field(value, "when", origin, &label)?;
    let Some(when) = When::parse(&when_str) else {
        return Err(format!(
            "{origin}: {label}: unknown \"when\" '{when_str}' \
             (expected satisfied, unsatisfied or always)"
        ));
    };
    let kind_str = string_field(value, "kind", origin, &label)?;
    let Some(kind) = MatchKind::parse(&kind_str) else {
        return Err(format!("{origin}: {label}: unknown \"kind\" '{kind_str}'"));
    };

    let text = string_field(value, "text", origin, &label)?;
    if text.contains('\n') {
        return Err(format!(
            "{origin}: {label}: \"text\" must not span lines; matching is line by line"
        ));
    }
    // Shape rules that would otherwise fail silently at match time.
    match kind {
        MatchKind::AtRule if !text.starts_with('@') => {
            return Err(format!(
                "{origin}: {label}: an at-rule pattern must start with '@'"
            ))
        }
        MatchKind::Function if !text.ends_with('(') => {
            return Err(format!(
                "{origin}: {label}: a function pattern must end with '('"
            ))
        }
        MatchKind::Declaration if !text.contains(':') => {
            return Err(format!(
                "{origin}: {label}: a declaration pattern must be written prop:value"
            ))
        }
        MatchKind::Unit if !text.chars().all(|c| c.is_ascii_alphabetic() || c == '%') => {
            return Err(format!(
                "{origin}: {label}: a unit pattern must be a bare unit name"
            ))
        }
        _ => {}
    }

    Ok(Detector {
        direction,
        when,
        kind,
        text,
        what: string_field(value, "what", origin, &label)?,
        action: optional_string_field(value, "action", origin, &label)?,
        scope: string_array_field(value, "scope", origin, &label)?,
    })
}

/// Reject members the schema does not define, so a typo is loud.
fn known_keys(value: &Value, allowed: &[&str], origin: &str, label: &str) -> Result<(), String> {
    let Some(members) = value.as_object() else {
        return Ok(());
    };
    for (key, _) in members {
        // The generator writes `//`-prefixed keys as file comments.
        if key.starts_with("//") || allowed.contains(&key.as_str()) {
            continue;
        }
        let where_ = if label.is_empty() {
            String::new()
        } else {
            format!("{label}: ")
        };
        return Err(format!(
            "{origin}: {where_}unknown field \"{key}\" (allowed: {})",
            allowed.join(", ")
        ));
    }
    Ok(())
}

fn string_field(value: &Value, key: &str, origin: &str, label: &str) -> Result<String, String> {
    match value.get(key) {
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(s.clone()),
        Some(Value::String(_)) => Err(field_err(origin, label, key, "must not be blank")),
        Some(other) => Err(field_err(
            origin,
            label,
            key,
            &format!("must be a string, found a {}", other.type_name()),
        )),
        None => Err(field_err(origin, label, key, "is required")),
    }
}

fn optional_string_field(
    value: &Value,
    key: &str,
    origin: &str,
    label: &str,
) -> Result<Option<String>, String> {
    match value.get(key) {
        None => Ok(None),
        Some(v) if v.is_null() => Ok(None),
        Some(_) => string_field(value, key, origin, label).map(Some),
    }
}

fn string_array_field(
    value: &Value,
    key: &str,
    origin: &str,
    label: &str,
) -> Result<Vec<String>, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item.as_str() {
                    Some(s) => out.push(s.to_string()),
                    None => return Err(field_err(origin, label, key, "must contain only strings")),
                }
            }
            Ok(out)
        }
        Some(other) => Err(field_err(
            origin,
            label,
            key,
            &format!("must be an array, found a {}", other.type_name()),
        )),
    }
}

fn count_field(value: &Value, key: &str, origin: &str, label: &str) -> Result<usize, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(0),
        Some(Value::Number(n)) if *n >= 0.0 && n.fract() == 0.0 => Ok(*n as usize),
        Some(_) => Err(field_err(
            origin,
            label,
            key,
            "must be a non-negative whole number",
        )),
    }
}

fn url_field(value: &Value, key: &str, origin: &str, label: &str) -> Result<String, String> {
    let s = string_field(value, key, origin, label)?;
    if !s.starts_with("https://") && !s.starts_with("http://") {
        return Err(field_err(origin, label, key, "must be an http(s) URL"));
    }
    Ok(s)
}

fn optional_url_field(
    value: &Value,
    key: &str,
    origin: &str,
    label: &str,
) -> Result<Option<String>, String> {
    match value.get(key) {
        None => Ok(None),
        Some(v) if v.is_null() => Ok(None),
        Some(_) => url_field(value, key, origin, label).map(Some),
    }
}

fn date_field(value: &Value, key: &str, origin: &str, label: &str) -> Result<Date, String> {
    let s = string_field(value, key, origin, label)?;
    Date::parse(&s).map_err(|e| field_err(origin, label, key, &e))
}

fn optional_date_field(
    value: &Value,
    key: &str,
    origin: &str,
    label: &str,
) -> Result<Option<Date>, String> {
    match value.get(key) {
        None => Ok(None),
        Some(v) if v.is_null() => Ok(None),
        Some(_) => date_field(value, key, origin, label).map(Some),
    }
}

fn field_err(origin: &str, label: &str, key: &str, what: &str) -> String {
    if label.is_empty() {
        format!("{origin}: \"{key}\" {what}")
    } else {
        format!("{origin}: {label}: \"{key}\" {what}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal(extra: &str, detectors: &str) -> String {
        format!(
            r#"{{
              "schema": 1,
              "dataset": {{
                "package": "web-features",
                "version": "3.0.0",
                "source_url": "https://registry.npmjs.org/web-features/-/web-features-3.0.0.tgz",
                "sha256": "abc",
                "retrieved_on": "2026-08-12",
                "generator": "tools/fetch-data.py"
              }},
              "features": [
                {{
                  "id": "has",
                  "name": ":has()",
                  "description": "A selector.",
                  "baseline": "high",
                  "baseline_low_date": "2023-12-19",
                  "baseline_high_date": "2026-06-19",
                  "risk_action": "Use a class."{extra},
                  "detectors": {detectors}
                }}
              ]
            }}"#
        )
    }

    const ONE_DETECTOR: &str = r#"[{"direction": "risk", "when": "unsatisfied", "kind": "selector",
             "text": ":has(", "what": "selector"}]"#;

    #[test]
    fn the_builtin_snapshot_parses_and_validates() {
        let c = Catalog::builtin();
        assert!(c.features.len() >= 40, "{}", c.features.len());
        assert_eq!(c.dataset.package, "web-features");
        for f in &c.features {
            assert!(!f.detectors.is_empty(), "{} has no detectors", f.id);
            assert!(
                f.compat_features_count > 0,
                "{} matched an empty dataset record",
                f.id
            );
            for d in &f.detectors {
                assert!(!d.text.is_empty(), "{} has an empty detector", f.id);
                let action = d.action.as_deref().unwrap_or(f.action(d.direction));
                assert!(action.len() > 20, "{}: action too thin: {action}", f.id);
            }
        }
    }

    #[test]
    fn the_snapshot_carries_the_motivating_features_with_their_real_dates() {
        // These come from web-features and must never be edited by hand. If the
        // dataset moves them, refresh the snapshot and update this test.
        let c = Catalog::builtin();
        let by_id = |id: &str| c.features.iter().find(|f| f.id == id).unwrap();

        let has = by_id("has");
        assert_eq!(has.name, ":has()");
        assert_eq!(has.baseline, Baseline::Widely);
        assert_eq!(has.baseline_low_date, Date::new(2023, 12, 19));
        assert_eq!(has.baseline_high_date, Date::new(2026, 6, 19));

        let cq = by_id("container-queries");
        assert_eq!(cq.baseline, Baseline::Widely);
        assert_eq!(cq.baseline_high_date, Date::new(2025, 8, 14));

        // backdrop-filter is Newly available only, so its -webkit- prefix is
        // NOT removable at the default target. This is the honesty test.
        let bf = by_id("backdrop-filter");
        assert_eq!(bf.baseline, Baseline::Newly);
        assert_eq!(bf.baseline_low_date, Date::new(2024, 9, 16));
        assert_eq!(bf.baseline_high_date, None);

        // user-select is not Baseline at all, so -webkit-user-select stays.
        let us = by_id("user-select");
        assert_eq!(us.baseline, Baseline::Limited);
        assert!(us.removable_action.is_none());
        assert!(us.detectors.iter().all(|d| d.direction == Direction::Risk));

        // anchor-positioning is Limited, so the Floating UI pointer must be an
        // "always" detector, never a "delete this now".
        let anchor = by_id("anchor-positioning");
        assert_eq!(anchor.baseline, Baseline::Limited);
        assert!(anchor
            .detectors
            .iter()
            .filter(|d| d.direction == Direction::Removable)
            .all(|d| d.when == When::Always));
    }

    #[test]
    fn no_removable_detector_points_at_a_limited_feature() {
        // A "you can delete this" finding is only ever justified by the feature
        // having reached the target. A Limited feature can never do that, so a
        // `satisfied` detector on one would be dead code -- or worse, a bug
        // that told someone to delete a working fallback.
        for f in &Catalog::builtin().features {
            if f.baseline != Baseline::Limited {
                continue;
            }
            for d in &f.detectors {
                assert!(
                    d.when != When::Satisfied,
                    "{}: a Limited feature must not have a 'satisfied' detector",
                    f.id
                );
            }
        }
    }

    #[test]
    fn features_are_sorted_by_id_and_unique() {
        let c = Catalog::builtin();
        let mut sorted = c.ids();
        sorted.sort_unstable();
        assert_eq!(c.ids(), sorted);
        let mut deduped = sorted.clone();
        deduped.dedup();
        assert_eq!(deduped, sorted);
    }

    #[test]
    fn every_detector_kind_is_exercised_by_the_snapshot() {
        // A kind nobody uses is a matcher rule nobody has checked.
        let c = Catalog::builtin();
        for kind in [
            MatchKind::AtRule,
            MatchKind::Property,
            MatchKind::PropertyStem,
            MatchKind::Declaration,
            MatchKind::Selector,
            MatchKind::Function,
            MatchKind::Unit,
            MatchKind::Ident,
            MatchKind::Package,
            MatchKind::Supports,
            MatchKind::SupportsSelector,
        ] {
            assert!(
                c.features
                    .iter()
                    .any(|f| f.detectors.iter().any(|d| d.kind == kind)),
                "no detector uses {}",
                kind.as_str()
            );
            assert_eq!(MatchKind::parse(kind.as_str()), Some(kind));
        }
    }

    #[test]
    fn optional_fields_default_sensibly() {
        let c = Catalog::from_json(&minimal("", ONE_DETECTOR), "t").unwrap();
        let f = &c.features[0];
        assert_eq!(f.spec, None);
        assert!(f.caniuse.is_empty());
        assert_eq!(f.compat_features_count, 0);
        assert_eq!(f.removable_action, None);
        assert!(f.detectors[0].scope.is_empty());
        assert_eq!(f.detectors[0].action, None);
    }

    #[test]
    fn rejects_broken_documents() {
        let cases: &[(&str, &str)] = &[
            (r#"{"schema": 2, "dataset": {}, "features": []}"#, "schema"),
            (r#"{"dataset": {}, "features": []}"#, "schema"),
            (r#"{"schema": 1, "features": []}"#, "dataset"),
            (r#"[]"#, "must be an object"),
        ];
        for (doc, needle) in cases {
            let err = Catalog::from_json(doc, "t").unwrap_err();
            assert!(err.contains(needle), "{doc} -> {err}");
        }
        assert!(Catalog::from_json(&minimal("", "[]"), "t")
            .unwrap_err()
            .contains("at least one detector"));
    }

    #[test]
    fn rejects_baseline_states_the_dataset_would_never_emit() {
        let cases: &[(&str, &str)] = &[
            (r#", "baseline": "yes""#, "unknown \"baseline\""),
            (
                r#", "baseline_high_date": null"#,
                "needs a \"baseline_high_date\"",
            ),
            (r#", "baseline": "false""#, "must not carry Baseline dates"),
            (r#", "baseline_high_date": "2020-01-01""#, "is before"),
            (r#", "baseline_low_date": "nope""#, "YYYY-MM-DD"),
            (r#", "id": "Has""#, "[a-z0-9-]"),
            (r#", "spec": "ftp://x""#, "http(s) URL"),
            (r#", "wat": 1"#, "unknown field"),
            (r#", "risk_action": null"#, "no \"risk_action\""),
            (r#", "caniuse": [3]"#, "only strings"),
            (r#", "compat_features_count": -1"#, "non-negative"),
        ];
        for (extra, needle) in cases {
            // Later duplicate members win, so an `extra` overrides the field of
            // the same name in the minimal document.
            let err = Catalog::from_json(&minimal(extra, ONE_DETECTOR), "t").unwrap_err();
            assert!(err.contains(needle), "{extra} -> {err}");
        }
    }

    #[test]
    fn rejects_detectors_whose_shape_could_never_match() {
        let cases: &[(&str, &str)] = &[
            (
                r#"[{"direction": "sideways", "when": "unsatisfied", "kind": "selector", "text": "x", "what": "w"}]"#,
                "unknown \"direction\"",
            ),
            (
                r#"[{"direction": "risk", "when": "maybe", "kind": "selector", "text": "x", "what": "w"}]"#,
                "unknown \"when\"",
            ),
            (
                r#"[{"direction": "risk", "when": "unsatisfied", "kind": "regex", "text": "x", "what": "w"}]"#,
                "unknown \"kind\"",
            ),
            (
                r#"[{"direction": "risk", "when": "unsatisfied", "kind": "at-rule", "text": "container", "what": "w"}]"#,
                "must start with '@'",
            ),
            (
                r#"[{"direction": "risk", "when": "unsatisfied", "kind": "function", "text": "oklch", "what": "w"}]"#,
                "must end with '('",
            ),
            (
                r#"[{"direction": "risk", "when": "unsatisfied", "kind": "declaration", "text": "balance", "what": "w"}]"#,
                "prop:value",
            ),
            (
                r#"[{"direction": "risk", "when": "unsatisfied", "kind": "unit", "text": "1lh", "what": "w"}]"#,
                "bare unit name",
            ),
            (
                r#"[{"direction": "risk", "when": "unsatisfied", "kind": "selector", "text": "a\nb", "what": "w"}]"#,
                "must not span lines",
            ),
            (
                r#"[{"direction": "risk", "when": "unsatisfied", "kind": "selector", "text": "x"}]"#,
                "\"what\" is required",
            ),
            (r#"["x"]"#, "must be an object"),
        ];
        for (detectors, needle) in cases {
            let err = Catalog::from_json(&minimal("", detectors), "t").unwrap_err();
            assert!(err.contains(needle), "{detectors} -> {err}");
        }
    }

    #[test]
    fn retain_applies_only_and_skip_and_rejects_typos() {
        let mut c = Catalog::builtin();
        c.retain(&["has".into(), "subgrid".into()], &["subgrid".into()])
            .unwrap();
        assert_eq!(c.ids(), ["has"]);

        let mut c = Catalog::builtin();
        let err = c.retain(&["nope".into()], &[]).unwrap_err();
        assert!(err.contains("unknown feature id 'nope'"), "{err}");
        assert!(err.contains("--list"), "{err}");
    }

    #[test]
    fn retain_direction_drops_detectors_then_empty_features() {
        let mut c = Catalog::builtin();
        let all = c.detector_count();
        c.retain_direction(Some(Direction::Removable));
        assert!(c.detector_count() < all);
        assert!(c.features.iter().all(|f| f
            .detectors
            .iter()
            .all(|d| d.direction == Direction::Removable)));
        // user-select is risk-only, so it must be gone entirely.
        assert!(!c.ids().contains(&"user-select"));

        let mut c = Catalog::builtin();
        c.retain_direction(None);
        assert_eq!(c.detector_count(), all);
    }

    #[test]
    fn when_fires_on_the_right_side_of_the_target() {
        assert!(When::Satisfied.fires(true));
        assert!(!When::Satisfied.fires(false));
        assert!(!When::Unsatisfied.fires(true));
        assert!(When::Unsatisfied.fires(false));
        assert!(When::Always.fires(true));
        assert!(When::Always.fires(false));
    }

    #[test]
    fn baseline_and_direction_round_trip_their_tokens() {
        for b in [Baseline::Limited, Baseline::Newly, Baseline::Widely] {
            assert_eq!(Baseline::parse(b.as_dataset_str()), Some(b));
            assert!(!b.label().is_empty());
            assert_eq!(b.short().len(), 7, "columns must line up");
        }
        for d in [Direction::Risk, Direction::Removable] {
            assert_eq!(Direction::parse(d.as_str()), Some(d));
        }
        for w in [When::Satisfied, When::Unsatisfied, When::Always] {
            assert_eq!(When::parse(w.as_str()), Some(w));
        }
        assert_eq!(Baseline::parse("HIGH"), None);
    }

    #[test]
    fn milestone_names_the_most_advanced_thing_that_happened() {
        let c = Catalog::builtin();
        let by_id = |id: &str| c.features.iter().find(|f| f.id == id).unwrap();
        assert_eq!(
            by_id("has").milestone(),
            Some(("widely since", Date::new(2026, 6, 19).unwrap()))
        );
        assert_eq!(
            by_id("backdrop-filter").milestone(),
            Some(("newly since", Date::new(2024, 9, 16).unwrap()))
        );
        assert_eq!(by_id("anchor-positioning").milestone(), None);
    }
}
