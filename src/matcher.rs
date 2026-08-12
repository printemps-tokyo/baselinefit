//! Textual matching of detectors against file contents.
//!
//! This is the honest core of the tool, and the honesty matters: baselinefit
//! does **not** parse CSS or JavaScript. It matches literal text, line by line,
//! with a per-kind boundary rule. A hit inside a comment, a string, a changelog
//! or a vendored stylesheet is a real hit as far as this module is concerned.
//! `--exclude` and the `baselinefit:ignore` marker exist because of that, not in
//! spite of it.
//!
//! # Boundary rules
//!
//! Each [`MatchKind`] pins down what "the text appears here" means, so that a
//! detector for one construct cannot fire on a different one that merely shares
//! a prefix. The rules, in full:
//!
//! | kind | fires when |
//! |---|---|
//! | `at-rule` | the literal appears, not preceded by an identifier character and not followed by one. `@container` therefore matches `@container (min-width: 20em)` and never `container-type: inline-size` or `@containerize`. |
//! | `property` | the literal is a complete CSS identifier followed by optional spaces and a `:`. `anchor-name` matches `anchor-name: --a` and `"anchor-name":` in JSON, but not `--anchor-name:` or `anchor-name-x:`. |
//! | `property-stem` | as `property`, except the identifier may continue after the literal with a `-` segment. `-webkit-mask` matches `-webkit-mask-image:`; `mask` matches `mask:` and `mask-image:` but not `masked:` and not `-webkit-mask-image:`. |
//! | `declaration` | a `property` match whose value, after the `:` and any spaces, is exactly the literal's value part. `text-wrap:balance` matches `text-wrap: balance;` but not `text-wrap: balanced`. |
//! | `selector` | the literal appears anywhere, with no boundary. `::view-transition` is deliberately a prefix, so it also covers `::view-transition-group`. |
//! | `function` | the literal (which ends in `(`) appears, not preceded by an identifier character. `oklch(` never fires inside `my-oklch(`. |
//! | `unit` | the literal directly follows a number, and is not followed by an identifier character. `lh` matches `1lh`, `.5lh` and `-1lh`, but not `line-height`, `1rlh` or `--gap-1lh`. |
//! | `ident` | the literal is a complete JavaScript identifier or dotted path: not preceded or followed by `[A-Za-z0-9_$]`. `ResizeObserver` does not fire on `MyResizeObserverShim`. A leading `.` is allowed, so `navigator.clipboard` matches `window.navigator.clipboard`. |
//! | `package` | the literal appears on a line that also contains a quote character, not preceded by `[A-Za-z0-9_@-]` and, when it ends in an identifier character, not followed by `[A-Za-z0-9_-]`. That covers imports, `require`, `package.json` dependencies and `<script src>`, and ignores the same name written in prose. |
//! | `supports` | the literal, read as a `property` or a `declaration`, appears inside an `@supports` condition on the same line. |
//! | `supports-selector` | the literal appears inside a `selector(...)` call inside an `@supports` condition on the same line. |
//!
//! Matching is case-sensitive throughout, because CSS property names, CSS
//! at-rules and JavaScript identifiers all are.
//!
//! # Guarded code is not risky code
//!
//! A feature named inside an `@supports` condition is being feature-detected,
//! not used unguarded. Risk detectors therefore ignore matches that fall inside
//! an `@supports` prelude on the same line: `@supports (backdrop-filter: blur(1px))`
//! is a guard, not a risk. The removable detectors are the ones that have
//! something to say about that line, and they say it only once the feature has
//! reached the target.
//!
//! Everything here is pure: strings in, [`Hit`]s out. No filesystem access.

use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::catalog::{Catalog, Direction, MatchKind};
use crate::target::Target;

/// Longest snippet rendered for a hit, in characters.
pub const SNIPPET_MAX: usize = 120;

/// The inline marker that suppresses findings on the line it appears on.
pub const IGNORE_MARKER: &str = "baselinefit:ignore";

/// The at-rule that introduces a feature query.
const SUPPORTS: &str = "@supports";

/// One detector that fired on a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchedDetector {
    /// The literal that matched.
    pub text: String,
    pub kind: MatchKind,
    /// The detector's short label, e.g. "vendor prefix".
    pub what: String,
    /// A detector-specific action, when it overrides the feature's.
    pub action: Option<String>,
    /// 1-based character column of the match.
    pub column: usize,
}

/// One line of one file that matched one feature, in one direction.
///
/// Several detectors from the same feature can fire on the same line (a
/// property and a unit, say); they are collected here rather than reported
/// separately, so the line is printed once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// Index into the [`Catalog`] the [`Matcher`] was built from.
    pub feature_index: usize,
    /// Which question this hit answers.
    pub direction: Direction,
    /// 1-based line number.
    pub line: usize,
    /// The trimmed, length-capped source line.
    pub snippet: String,
    /// Detectors that fired, ordered by column then by text.
    pub detectors: Vec<MatchedDetector>,
}

impl Hit {
    /// 1-based column of the leftmost match on the line.
    pub fn column(&self) -> usize {
        self.detectors.first().map_or(1, |d| d.column)
    }
}

/// Every hit in one file, keyed by its repository-relative path.
pub type FileHits = (String, Vec<Hit>);

/// A catalog resolved against a target, ready to run over file contents.
///
/// Resolution happens once, here: a detector whose [`crate::catalog::When`]
/// cannot fire for its feature at this target is dropped, so `scan` never has
/// to reason about Baseline again. That is what makes `--target` flip the same
/// input between "risk" and "fine".
pub struct Matcher {
    features: Vec<CompiledFeature>,
    /// Every feature id, used to interpret a scoped `baselinefit:ignore`.
    ids: Vec<String>,
}

struct CompiledFeature {
    index: usize,
    id: String,
    detectors: Vec<CompiledDetector>,
}

struct CompiledDetector {
    direction: Direction,
    kind: MatchKind,
    text: String,
    what: String,
    action: Option<String>,
    /// `None` when the detector applies to every scanned file.
    scope: Option<Gitignore>,
}

impl Matcher {
    /// Compile `catalog` for `target`.
    ///
    /// Fails only if a detector's `scope` contains a glob the gitignore glob
    /// engine rejects.
    pub fn new(catalog: &Catalog, target: Target) -> Result<Matcher, String> {
        let mut features = Vec::with_capacity(catalog.features.len());
        for (index, feature) in catalog.features.iter().enumerate() {
            let satisfied = target.satisfied_by(feature);
            let mut detectors = Vec::new();
            for detector in &feature.detectors {
                if !detector.when.fires(satisfied) {
                    continue;
                }
                let scope = if detector.scope.is_empty() {
                    None
                } else {
                    Some(compile_globs(&detector.scope).map_err(|e| {
                        format!("feature '{}': invalid scope glob: {e}", feature.id)
                    })?)
                };
                detectors.push(CompiledDetector {
                    direction: detector.direction,
                    kind: detector.kind,
                    text: detector.text.clone(),
                    what: detector.what.clone(),
                    action: detector.action.clone(),
                    scope,
                });
            }
            features.push(CompiledFeature {
                index,
                id: feature.id.clone(),
                detectors,
            });
        }
        let ids = catalog.features.iter().map(|f| f.id.clone()).collect();
        Ok(Matcher { features, ids })
    }

    /// Total number of detectors that can fire at this target.
    pub fn live_detectors(&self) -> usize {
        self.features.iter().map(|f| f.detectors.len()).sum()
    }

    /// Run every live detector over `content`.
    ///
    /// `rel_path` is the repository-relative path with forward slashes; it is
    /// only used to evaluate detector scopes. The returned hits are ordered by
    /// feature index, then direction, then line.
    pub fn scan(&self, rel_path: &str, content: &str) -> Vec<Hit> {
        let mut hits = Vec::new();
        for feature in &self.features {
            let applicable: Vec<&CompiledDetector> = feature
                .detectors
                .iter()
                .filter(|d| in_scope(d, rel_path))
                .collect();
            if applicable.is_empty() {
                continue;
            }
            for direction in [Direction::Risk, Direction::Removable] {
                for (i, raw) in content.lines().enumerate() {
                    // `lines()` strips \n but leaves \r on CRLF input.
                    let line = raw.strip_suffix('\r').unwrap_or(raw);
                    if Suppression::parse(line, &self.ids).covers(&feature.id) {
                        continue;
                    }
                    let guard = supports_prelude(line);
                    let mut matched: Vec<MatchedDetector> = Vec::new();
                    for d in applicable.iter().filter(|d| d.direction == direction) {
                        let Some(offset) = find_first(line, &d.text, d.kind) else {
                            continue;
                        };
                        // A feature named inside an @supports condition is
                        // being detected, not used unguarded.
                        if direction == Direction::Risk
                            && guard.is_some_and(|(s, e)| offset >= s && offset < e)
                        {
                            continue;
                        }
                        matched.push(MatchedDetector {
                            text: d.text.clone(),
                            kind: d.kind,
                            what: d.what.clone(),
                            action: d.action.clone(),
                            column: line[..offset].chars().count() + 1,
                        });
                    }
                    if matched.is_empty() {
                        continue;
                    }
                    matched
                        .sort_by(|a, b| a.column.cmp(&b.column).then_with(|| a.text.cmp(&b.text)));
                    let focus = matched[0].column;
                    hits.push(Hit {
                        feature_index: feature.index,
                        direction,
                        line: i + 1,
                        snippet: snippet(line, focus),
                        detectors: matched,
                    });
                }
            }
        }
        hits
    }
}

fn in_scope(detector: &CompiledDetector, rel_path: &str) -> bool {
    match &detector.scope {
        None => true,
        Some(globs) => globs.matched(rel_path, false).is_ignore(),
    }
}

fn compile_globs(globs: &[String]) -> Result<Gitignore, ignore::Error> {
    // An empty root makes the matcher operate directly on the relative paths
    // handed to `matched`, which is what the scanner already produces.
    let mut builder = GitignoreBuilder::new("");
    for glob in globs {
        builder.add_line(None, glob)?;
    }
    builder.build()
}

// ---------------------------------------------------------------------------
// Character classes
// ---------------------------------------------------------------------------

/// A character that can appear inside a CSS identifier. Non-ASCII counts:
/// CSS identifiers may contain any code point above U+0080.
fn is_css_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_' || !c.is_ascii()
}

/// A character that can appear inside a JavaScript identifier.
fn is_js_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$' || !c.is_ascii()
}

/// A character that continues a package or file name.
fn is_pkg_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

fn char_before(line: &str, at: usize) -> Option<char> {
    line[..at].chars().next_back()
}

fn char_at(line: &str, at: usize) -> Option<char> {
    line[at..].chars().next()
}

/// Skip spaces and tabs, returning the next byte offset.
fn skip_spaces(line: &str, mut at: usize) -> usize {
    while let Some(c) = char_at(line, at) {
        if c == ' ' || c == '\t' {
            at += c.len_utf8();
        } else {
            break;
        }
    }
    at
}

/// Byte offset just past a `:` following `at`, if one is there.
///
/// A single closing quote may sit between the two, so that an object key such
/// as `"backdrop-filter": "blur(4px)"` in JSON or a JavaScript style object
/// reads as the declaration it is.
fn colon_after(line: &str, at: usize) -> Option<usize> {
    let mut at = skip_spaces(line, at);
    if matches!(char_at(line, at), Some('"') | Some('\'')) {
        at = skip_spaces(line, at + 1);
    }
    if char_at(line, at) == Some(':') {
        Some(at + 1)
    } else {
        None
    }
}

/// Advance past a run of CSS identifier characters.
fn ident_end(line: &str, mut at: usize) -> usize {
    while let Some(c) = char_at(line, at) {
        if is_css_ident(c) {
            at += c.len_utf8();
        } else {
            break;
        }
    }
    at
}

/// True when the nearest non-whitespace character before `at` starts a
/// selector rather than a declaration. Keeps a `property` detector off
/// `.mask:hover` while leaving `{ mask: none }` and `;mask:none` alone.
fn in_selector(line: &str, at: usize) -> bool {
    let Some(c) = line[..at].chars().rev().find(|c| !c.is_whitespace()) else {
        return false;
    };
    matches!(c, '.' | '#' | '&' | '[')
}

// ---------------------------------------------------------------------------
// Per-kind matching
// ---------------------------------------------------------------------------

/// Find the first occurrence of `text` in `line` that satisfies `kind`'s
/// boundary rule, returning its byte offset. See the module documentation for
/// the rule each kind applies.
pub fn find_first(line: &str, text: &str, kind: MatchKind) -> Option<usize> {
    if text.is_empty() {
        return None;
    }
    match kind {
        MatchKind::AtRule => find_at_rule(line, text),
        MatchKind::Property => find_property(line, text),
        MatchKind::PropertyStem => find_property_stem(line, text),
        MatchKind::Declaration => find_declaration(line, text),
        MatchKind::Selector => line.find(text),
        MatchKind::Function => find_function(line, text),
        MatchKind::Unit => find_unit(line, text),
        MatchKind::Ident => find_ident(line, text),
        MatchKind::Package => find_package(line, text),
        MatchKind::Supports => find_in_supports(line, text),
        MatchKind::SupportsSelector => find_in_supports_selector(line, text),
    }
}

fn find_at_rule(line: &str, text: &str) -> Option<usize> {
    for (offset, _) in line.match_indices(text) {
        if char_before(line, offset).is_some_and(|c| is_css_ident(c) || c == '@') {
            continue;
        }
        if char_at(line, offset + text.len()).is_some_and(is_css_ident) {
            continue;
        }
        return Some(offset);
    }
    None
}

fn find_property(line: &str, text: &str) -> Option<usize> {
    property_match(line, text, false).map(|(offset, _)| offset)
}

fn find_property_stem(line: &str, text: &str) -> Option<usize> {
    property_match(line, text, true).map(|(offset, _)| offset)
}

/// Match a property name, returning `(offset of the name, offset just past its
/// colon)`. With `stem`, the identifier may continue with further `-` segments.
fn property_match(line: &str, text: &str, stem: bool) -> Option<(usize, usize)> {
    for (offset, _) in line.match_indices(text) {
        if char_before(line, offset).is_some_and(is_css_ident) {
            continue;
        }
        if in_selector(line, offset) {
            continue;
        }
        let after_text = offset + text.len();
        let end = match char_at(line, after_text) {
            // The identifier ends exactly where the literal does.
            None => after_text,
            Some(c) if !is_css_ident(c) => after_text,
            // A stem may run on, but only into a further `-` segment.
            Some('-') if stem => ident_end(line, after_text),
            _ => continue,
        };
        let Some(past_colon) = colon_after(line, end) else {
            continue;
        };
        return Some((offset, past_colon));
    }
    None
}

fn find_declaration(line: &str, text: &str) -> Option<usize> {
    // The catalog validator guarantees the `prop:value` shape.
    let (prop, value) = text.split_once(':')?;
    let prop = prop.trim();
    let value = value.trim();
    let mut from = 0;
    while let Some((offset, past_colon)) = property_match(&line[from..], prop, false) {
        let offset = offset + from;
        let past_colon = past_colon + from;
        let start = skip_spaces(line, past_colon);
        if line[start..].starts_with(value) {
            let end = start + value.len();
            if !char_at(line, end).is_some_and(is_css_ident) {
                return Some(offset);
            }
        }
        from = offset + prop.len();
        if from >= line.len() {
            break;
        }
    }
    None
}

fn find_function(line: &str, text: &str) -> Option<usize> {
    for (offset, _) in line.match_indices(text) {
        if char_before(line, offset).is_some_and(is_css_ident) {
            continue;
        }
        return Some(offset);
    }
    None
}

fn find_unit(line: &str, text: &str) -> Option<usize> {
    for (offset, _) in line.match_indices(text) {
        if char_at(line, offset + text.len()).is_some_and(|c| is_css_ident(c) || c == '%') {
            continue;
        }
        // Walk back over the numeric literal the unit must be attached to.
        let mut at = offset;
        let mut saw_digit = false;
        while let Some(c) = char_before(line, at) {
            if c.is_ascii_digit() {
                saw_digit = true;
                at -= 1;
            } else if c == '.' {
                at -= 1;
            } else {
                break;
            }
        }
        if !saw_digit {
            continue;
        }
        // An optional sign belongs to the number, not to a surrounding word.
        if char_before(line, at).is_some_and(|c| c == '-' || c == '+') {
            at -= 1;
        }
        if char_before(line, at).is_some_and(|c| is_css_ident(c) || c == '.') {
            continue;
        }
        return Some(offset);
    }
    None
}

fn find_ident(line: &str, text: &str) -> Option<usize> {
    for (offset, _) in line.match_indices(text) {
        if char_before(line, offset).is_some_and(is_js_ident) {
            continue;
        }
        if char_at(line, offset + text.len()).is_some_and(is_js_ident) {
            continue;
        }
        return Some(offset);
    }
    None
}

fn find_package(line: &str, text: &str) -> Option<usize> {
    // Imports, `require`, package.json dependencies and `<script src>` all put
    // the name in quotes. A mention in prose does not, and is not a reference.
    if !line.contains(['"', '\'', '`']) {
        return None;
    }
    let ends_in_ident = text.chars().next_back().is_some_and(is_pkg_ident);
    for (offset, _) in line.match_indices(text) {
        if char_before(line, offset).is_some_and(|c| is_pkg_ident(c) || c == '@') {
            continue;
        }
        if ends_in_ident && char_at(line, offset + text.len()).is_some_and(is_pkg_ident) {
            continue;
        }
        return Some(offset);
    }
    None
}

/// The byte range of an `@supports` condition on this line: from just after the
/// at-rule to the `{` that opens its block, or to the end of the line.
pub fn supports_prelude(line: &str) -> Option<(usize, usize)> {
    let at = find_at_rule(line, SUPPORTS)?;
    let start = at + SUPPORTS.len();
    let end = line[start..]
        .find('{')
        .map_or(line.len(), |offset| start + offset);
    Some((start, end))
}

fn find_in_supports(line: &str, text: &str) -> Option<usize> {
    let (start, end) = supports_prelude(line)?;
    let region = &line[start..end];
    let found = if text.contains(':') {
        find_declaration(region, text)
    } else {
        find_property(region, text)
    };
    found.map(|offset| offset + start)
}

fn find_in_supports_selector(line: &str, text: &str) -> Option<usize> {
    let (start, end) = supports_prelude(line)?;
    let region = &line[start..end];
    let call = find_function(region, "selector(")?;
    let from = call + "selector(".len();
    region[from..]
        .find(text)
        .map(|offset| offset + from + start)
}

// ---------------------------------------------------------------------------
// Suppression
// ---------------------------------------------------------------------------

/// What an inline `baselinefit:ignore` marker on a line suppresses.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Suppression {
    /// No marker on this line.
    None,
    /// A bare marker: everything on this line is suppressed.
    All,
    /// `baselinefit:ignore <id>[, <id>]`: only those features are suppressed.
    Only(Vec<String>),
}

impl Suppression {
    /// Read the marker, if any.
    ///
    /// The tokens after the marker are treated as feature ids only when every
    /// one of them names a feature in the catalog. Anything else -- prose, a
    /// reason, a mistyped id -- degrades to the bare form and suppresses the
    /// whole line, which is the behavior a reader of the comment expects.
    fn parse(line: &str, known_ids: &[String]) -> Suppression {
        let Some(at) = line.find(IGNORE_MARKER) else {
            return Suppression::None;
        };
        let rest = &line[at + IGNORE_MARKER.len()..];
        let mut ids = Vec::new();
        for token in rest.split([',', ' ', '\t']) {
            let token = token.trim().trim_end_matches(['*', '/', '-']);
            if token.is_empty() {
                continue;
            }
            if !known_ids.iter().any(|id| id == token) {
                return Suppression::All;
            }
            ids.push(token.to_string());
        }
        if ids.is_empty() {
            Suppression::All
        } else {
            Suppression::Only(ids)
        }
    }

    fn covers(&self, feature_id: &str) -> bool {
        match self {
            Suppression::None => false,
            Suppression::All => true,
            Suppression::Only(ids) => ids.iter().any(|id| id == feature_id),
        }
    }
}

/// Trim a line and cap it at [`SNIPPET_MAX`] characters, keeping the character
/// at `focus` (a 1-based column) visible.
fn snippet(line: &str, focus: usize) -> String {
    let chars: Vec<char> = line.chars().collect();
    let start = chars.iter().position(|c| !c.is_whitespace()).unwrap_or(0);
    let end = chars
        .iter()
        .rposition(|c| !c.is_whitespace())
        .map_or(start, |i| i + 1);
    let trimmed = &chars[start..end];
    if trimmed.len() <= SNIPPET_MAX {
        return trimmed.iter().collect();
    }

    // Long line: slide a window so the match sits a third of the way in.
    let focus_in_trimmed = focus.saturating_sub(1).saturating_sub(start);
    let lead = SNIPPET_MAX / 3;
    let mut from = focus_in_trimmed.saturating_sub(lead);
    if from + SNIPPET_MAX > trimmed.len() {
        from = trimmed.len() - SNIPPET_MAX;
    }
    let to = from + SNIPPET_MAX;

    let mut out = String::new();
    if from > 0 {
        out.push_str("...");
    }
    out.extend(&trimmed[from..to]);
    if to < trimmed.len() {
        out.push_str("...");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;

    fn find(line: &str, text: &str, kind: MatchKind) -> Option<usize> {
        find_first(line, text, kind)
    }

    #[test]
    fn an_at_rule_never_fires_on_a_property_that_shares_its_name() {
        // The motivating case: @container and container-type are different
        // things, and confusing them would misreport every container query.
        assert_eq!(
            find(
                "container-type: inline-size;",
                "@container",
                MatchKind::AtRule
            ),
            None
        );
        assert_eq!(
            find(
                "  @container (min-width: 20em) {",
                "@container",
                MatchKind::AtRule
            ),
            Some(2)
        );
        assert_eq!(
            find(
                "@container card (min-width: 20em) {",
                "@container",
                MatchKind::AtRule
            ),
            Some(0)
        );
        assert_eq!(
            find("@containerize {", "@container", MatchKind::AtRule),
            None
        );
        assert_eq!(
            find("@layer base, utilities;", "@layer", MatchKind::AtRule),
            Some(0)
        );
        assert_eq!(find("@layers {", "@layer", MatchKind::AtRule), None);
        // A prefixed at-rule is its own literal and must not be split.
        assert_eq!(
            find(
                "@-webkit-keyframes spin {",
                "@-webkit-keyframes",
                MatchKind::AtRule
            ),
            Some(0)
        );
    }

    #[test]
    fn a_property_must_be_a_whole_identifier_followed_by_a_colon() {
        assert_eq!(
            find("  anchor-name: --tip;", "anchor-name", MatchKind::Property),
            Some(2)
        );
        assert_eq!(
            find("anchor-name:--tip", "anchor-name", MatchKind::Property),
            Some(0)
        );
        assert_eq!(
            find("  anchor-name : --tip;", "anchor-name", MatchKind::Property),
            Some(2)
        );
        // A JSON or JS style object still reads as a declaration.
        assert_eq!(
            find(
                r#"{"anchor-name": "--tip"}"#,
                "anchor-name",
                MatchKind::Property
            ),
            Some(2)
        );
        // Longer identifiers, custom properties and bare mentions do not.
        assert_eq!(
            find("anchor-name-x: 1;", "anchor-name", MatchKind::Property),
            None
        );
        assert_eq!(
            find("--anchor-name: 1;", "anchor-name", MatchKind::Property),
            None
        );
        assert_eq!(
            find("x-anchor-name: 1;", "anchor-name", MatchKind::Property),
            None
        );
        assert_eq!(
            find(
                "/* anchor-name is nice */",
                "anchor-name",
                MatchKind::Property
            ),
            None
        );
        // A class selector is not a declaration.
        assert_eq!(
            find(".user-select:hover {", "user-select", MatchKind::Property),
            None
        );
        assert_eq!(
            find("#user-select:hover {", "user-select", MatchKind::Property),
            None
        );
        assert_eq!(
            find("&.user-select:focus {", "user-select", MatchKind::Property),
            None
        );
        assert_eq!(
            find(
                "p { user-select: none; }",
                "user-select",
                MatchKind::Property
            ),
            Some(4)
        );
    }

    #[test]
    fn a_property_stem_covers_the_longhands_but_not_a_longer_prefix() {
        assert_eq!(
            find(
                "-webkit-mask-image: url(a.svg);",
                "-webkit-mask",
                MatchKind::PropertyStem
            ),
            Some(0)
        );
        assert_eq!(
            find(
                "-webkit-mask: none;",
                "-webkit-mask",
                MatchKind::PropertyStem
            ),
            Some(0)
        );
        assert_eq!(
            find("mask-image: url(a.svg);", "mask", MatchKind::PropertyStem),
            Some(0)
        );
        // The unprefixed stem must not reach into the prefixed property.
        assert_eq!(
            find(
                "  -webkit-mask-image: url(a.svg);",
                "mask",
                MatchKind::PropertyStem
            ),
            None
        );
        // A stem only runs on into a further `-` segment.
        assert_eq!(find("masked: 1;", "mask", MatchKind::PropertyStem), None);
        assert_eq!(
            find(
                "-ms-flex-align: center;",
                "-ms-flex",
                MatchKind::PropertyStem
            ),
            Some(0)
        );
    }

    #[test]
    fn a_declaration_pins_the_value_as_well_as_the_property() {
        assert_eq!(
            find(
                "text-wrap: balance;",
                "text-wrap:balance",
                MatchKind::Declaration
            ),
            Some(0)
        );
        assert_eq!(
            find(
                "  text-wrap:balance",
                "text-wrap:balance",
                MatchKind::Declaration
            ),
            Some(2)
        );
        assert_eq!(
            find(
                "text-wrap: balance !important;",
                "text-wrap:balance",
                MatchKind::Declaration
            ),
            Some(0)
        );
        assert_eq!(
            find(
                "text-wrap: balanced;",
                "text-wrap:balance",
                MatchKind::Declaration
            ),
            None
        );
        assert_eq!(
            find(
                "text-wrap: pretty;",
                "text-wrap:balance",
                MatchKind::Declaration
            ),
            None
        );
        assert_eq!(
            find(
                "display: -webkit-flex;",
                "display:-webkit-flex",
                MatchKind::Declaration
            ),
            Some(0)
        );
        assert_eq!(
            find(
                "grid-template-columns: subgrid;",
                "grid-template-columns:subgrid",
                MatchKind::Declaration
            ),
            Some(0)
        );
        // A second occurrence of the property carries the value we want.
        assert_eq!(
            find(
                "display: block; display: -webkit-flex;",
                "display:-webkit-flex",
                MatchKind::Declaration
            ),
            Some(16)
        );
    }

    #[test]
    fn a_function_needs_a_clean_left_boundary() {
        assert_eq!(
            find("color: oklch(70% 0.1 200);", "oklch(", MatchKind::Function),
            Some(7)
        );
        assert_eq!(
            find("color: my-oklch(1);", "oklch(", MatchKind::Function),
            None
        );
        assert_eq!(
            find(
                "background: color-mix(in oklab, red, blue);",
                "color-mix(",
                MatchKind::Function
            ),
            Some(12)
        );
        assert_eq!(
            find(
                "@import url(a.css) layer(base);",
                "layer(",
                MatchKind::Function
            ),
            Some(19)
        );
    }

    #[test]
    fn a_unit_must_be_attached_to_a_number() {
        assert_eq!(find("height: 100dvh;", "dvh", MatchKind::Unit), Some(11));
        assert_eq!(find("margin: -1lh 0;", "lh", MatchKind::Unit), Some(10));
        assert_eq!(find("margin: .5lh;", "lh", MatchKind::Unit), Some(10));
        assert_eq!(
            find("width: calc(100cqw - 1rem);", "cqw", MatchKind::Unit),
            Some(15)
        );
        // Not attached to a number, or attached to a longer unit or word.
        assert_eq!(find("line-height: 1.5;", "lh", MatchKind::Unit), None);
        assert_eq!(find("margin: 1rlh;", "lh", MatchKind::Unit), None);
        assert_eq!(find("margin: 1rlh;", "rlh", MatchKind::Unit), Some(9));
        assert_eq!(
            find("padding: var(--gap-1lh);", "lh", MatchKind::Unit),
            None
        );
        assert_eq!(find("width: 100dvhx;", "dvh", MatchKind::Unit), None);
        assert_eq!(find("width: 100dvh%;", "dvh", MatchKind::Unit), None);
    }

    #[test]
    fn a_js_identifier_does_not_fire_inside_a_longer_one() {
        assert_eq!(
            find("new SharedWorker('w.js')", "SharedWorker", MatchKind::Ident),
            Some(4)
        );
        assert_eq!(
            find(
                "class SharedWorkerShim {}",
                "SharedWorker",
                MatchKind::Ident
            ),
            None
        );
        assert_eq!(
            find("const MySharedWorker = 1", "SharedWorker", MatchKind::Ident),
            None
        );
        assert_eq!(
            find("el.showPopover()", "showPopover", MatchKind::Ident),
            Some(3)
        );
        assert_eq!(
            find(
                "window.navigator.clipboard.writeText(x)",
                "navigator.clipboard",
                MatchKind::Ident
            ),
            Some(7)
        );
        assert_eq!(
            find("structuredClone(x)", "structuredClone", MatchKind::Ident),
            Some(0)
        );
        assert_eq!(
            find("structuredCloned(x)", "structuredClone", MatchKind::Ident),
            None
        );
        // Case sensitivity is the whole point for JS identifiers.
        assert_eq!(find("sharedworker", "SharedWorker", MatchKind::Ident), None);
    }

    #[test]
    fn a_package_reference_needs_a_quote_on_the_line() {
        let text = "resize-observer-polyfill";
        assert_eq!(
            find(
                "import RO from 'resize-observer-polyfill';",
                text,
                MatchKind::Package
            ),
            Some(16)
        );
        assert_eq!(
            find(
                r#"  "resize-observer-polyfill": "^1.5.1","#,
                text,
                MatchKind::Package
            ),
            Some(3)
        );
        assert_eq!(
            find(
                "require('resize-observer-polyfill/dist/ResizeObserver')",
                text,
                MatchKind::Package
            ),
            Some(9)
        );
        // Prose is not a reference.
        assert_eq!(
            find(
                "// we dropped resize-observer-polyfill last year",
                text,
                MatchKind::Package
            ),
            None
        );
        // A different package that merely contains the name is not it.
        assert_eq!(
            find(
                "import x from 'my-resize-observer-polyfill';",
                text,
                MatchKind::Package
            ),
            None
        );
        assert_eq!(
            find(
                "import x from 'resize-observer-polyfill-2';",
                text,
                MatchKind::Package
            ),
            None
        );
        // A scoped prefix ends in `/`, so anything may follow it.
        assert_eq!(
            find(
                "import {computePosition} from '@floating-ui/dom';",
                "@floating-ui/",
                MatchKind::Package
            ),
            Some(31)
        );
        assert_eq!(
            find(
                r#"<script src="js/focus-visible.js"></script>"#,
                "focus-visible.js",
                MatchKind::Package
            ),
            Some(16)
        );
        assert_eq!(
            find(r#"import "popper.json";"#, "popper.js", MatchKind::Package),
            None
        );
    }

    #[test]
    fn supports_detectors_only_fire_inside_a_feature_query() {
        assert_eq!(
            find(
                "@supports (aspect-ratio: 1/1) {",
                "aspect-ratio",
                MatchKind::Supports
            ),
            Some(11)
        );
        assert_eq!(
            find(
                "@supports not (aspect-ratio: 1/1) {",
                "aspect-ratio",
                MatchKind::Supports
            ),
            Some(15)
        );
        assert_eq!(
            find("aspect-ratio: 1/1;", "aspect-ratio", MatchKind::Supports),
            None
        );
        // Past the brace is the body, not the condition.
        assert_eq!(
            find(
                "@supports (gap: 1px) { .x { aspect-ratio: 1; } }",
                "aspect-ratio",
                MatchKind::Supports
            ),
            None
        );
        assert_eq!(
            find(
                "@supports (display: flex) {",
                "display:flex",
                MatchKind::Supports
            ),
            Some(11)
        );
        assert_eq!(
            find(
                "@supports (display: grid) {",
                "display:flex",
                MatchKind::Supports
            ),
            None
        );
        assert_eq!(
            find("@supports (row-gap: 1px) {", "gap", MatchKind::Supports),
            None
        );
        assert_eq!(
            find("@supports (gap: 1px) {", "gap", MatchKind::Supports),
            Some(11)
        );
    }

    #[test]
    fn supports_selector_detectors_need_the_selector_call() {
        assert_eq!(
            find(
                "@supports selector(:has(a)) {",
                ":has(",
                MatchKind::SupportsSelector
            ),
            Some(19)
        );
        assert_eq!(
            find(
                "@supports selector(:focus-visible) {",
                ":focus-visible",
                MatchKind::SupportsSelector
            ),
            Some(19)
        );
        assert_eq!(
            find("li:has(a) {", ":has(", MatchKind::SupportsSelector),
            None
        );
        assert_eq!(
            find(
                "@supports (display: flex) { li:has(a) {} }",
                ":has(",
                MatchKind::SupportsSelector
            ),
            None
        );
    }

    #[test]
    fn a_selector_fragment_is_a_prefix_on_purpose() {
        assert_eq!(
            find("li:has(> img) {", ":has(", MatchKind::Selector),
            Some(2)
        );
        assert_eq!(
            find(
                "::view-transition-group(root) {",
                "::view-transition",
                MatchKind::Selector
            ),
            Some(0)
        );
        assert_eq!(
            find(
                "::view-transition {",
                "::view-transition",
                MatchKind::Selector
            ),
            Some(0)
        );
    }

    // -----------------------------------------------------------------------
    // Matcher-level behavior
    // -----------------------------------------------------------------------

    fn catalog() -> Catalog {
        Catalog::builtin()
    }

    fn ids_at(target: Target, path: &str, content: &str) -> Vec<(String, &'static str)> {
        let cat = catalog();
        let m = Matcher::new(&cat, target).unwrap();
        let mut out: Vec<(String, &'static str)> = m
            .scan(path, content)
            .into_iter()
            .map(|h| {
                (
                    cat.features[h.feature_index].id.clone(),
                    h.direction.as_str(),
                )
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    #[test]
    fn the_target_flips_the_same_line_between_risk_and_silence() {
        // :has() became Newly available 2023-12-19 and Widely 2026-06-19.
        let css = "li:has(> img) { color: red; }\n";
        assert!(ids_at(Target::Year(2022), "a.css", css).contains(&("has".into(), "risk")));
        assert!(ids_at(Target::Year(2024), "a.css", css).is_empty());
        assert!(ids_at(Target::Widely, "a.css", css).is_empty());
    }

    #[test]
    fn the_target_flips_a_prefix_between_removable_and_load_bearing() {
        // backdrop-filter is Newly available (2024-09-16), not Widely.
        let css = "-webkit-backdrop-filter: blur(4px);\n";
        assert!(ids_at(Target::Widely, "a.css", css).is_empty());
        assert!(
            ids_at(Target::Newly, "a.css", css).contains(&("backdrop-filter".into(), "removable"))
        );
    }

    #[test]
    fn a_dead_supports_guard_is_removable_and_a_live_one_is_silent() {
        let css = "@supports (display: grid) { .g { display: grid; } }\n";
        assert!(ids_at(Target::Widely, "a.css", css).contains(&("grid".into(), "removable")));
        assert!(ids_at(Target::Year(2015), "a.css", css).is_empty());
    }

    #[test]
    fn a_guarded_use_is_not_reported_as_a_risk() {
        // The property appears inside the condition, so it is being detected,
        // not used. Nothing to warn about at any target.
        let css = "@supports (backdrop-filter: blur(1px)) {\n";
        assert!(ids_at(Target::Widely, "a.css", css).is_empty());
        assert!(ids_at(Target::Year(2015), "a.css", css).is_empty());
        // The same declaration outside a guard is a risk at a strict target.
        let plain = "  backdrop-filter: blur(1px);\n";
        assert!(
            ids_at(Target::Widely, "a.css", plain).contains(&("backdrop-filter".into(), "risk"))
        );
    }

    #[test]
    fn a_forward_pointer_fires_at_every_target_and_stays_removable() {
        // anchor-positioning is Limited, so the Floating UI pointer must still
        // appear -- as information, never as "delete this".
        let js = "import {computePosition} from '@floating-ui/dom';\n";
        for target in [Target::Widely, Target::Newly, Target::Year(2015)] {
            assert!(
                ids_at(target, "a.js", js).contains(&("anchor-positioning".into(), "removable")),
                "{target:?}"
            );
        }
    }

    #[test]
    fn several_detectors_on_one_line_collapse_into_one_hit() {
        let cat = catalog();
        let m = Matcher::new(&cat, Target::Year(2015)).unwrap();
        let hits = m.scan("a.css", "  width: calc(100cqw - 1cqh);");
        assert_eq!(hits.len(), 1, "{hits:#?}");
        let texts: Vec<&str> = hits[0].detectors.iter().map(|d| d.text.as_str()).collect();
        assert_eq!(texts, ["cqw", "cqh"]);
        assert_eq!(hits[0].column(), 18);
    }

    #[test]
    fn one_feature_can_produce_both_directions_on_one_file() {
        let cat = catalog();
        let m = Matcher::new(&cat, Target::Widely).unwrap();
        let hits = m.scan(
            "a.js",
            "import {computePosition} from '@floating-ui/dom';\n.tip { anchor-name: --a; }\n",
        );
        let dirs: Vec<Direction> = hits.iter().map(|h| h.direction).collect();
        assert!(dirs.contains(&Direction::Risk), "{hits:#?}");
        assert!(dirs.contains(&Direction::Removable), "{hits:#?}");
    }

    #[test]
    fn line_numbers_are_one_based_and_survive_crlf() {
        let cat = catalog();
        let m = Matcher::new(&cat, Target::Year(2015)).unwrap();
        let hits = m.scan("a.css", "a{}\r\nb{}\r\n@scope (.card) {\r\n");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line, 3);
        assert_eq!(hits[0].snippet, "@scope (.card) {");
    }

    #[test]
    fn columns_count_characters_not_bytes() {
        let cat = catalog();
        let m = Matcher::new(&cat, Target::Year(2015)).unwrap();
        let hits = m.scan("a.js", "// 日本語 new SharedWorker('w')");
        assert_eq!(hits[0].column(), 12);
    }

    #[test]
    fn a_bare_ignore_marker_suppresses_the_whole_line() {
        let cat = catalog();
        let m = Matcher::new(&cat, Target::Year(2015)).unwrap();
        assert!(m
            .scan("a.css", "@scope (.card) { } /* baselinefit:ignore */")
            .is_empty());
        assert_eq!(m.scan("a.css", "@scope (.card) { }").len(), 1);
    }

    #[test]
    fn a_scoped_ignore_marker_only_suppresses_the_named_feature() {
        let cat = catalog();
        let m = Matcher::new(&cat, Target::Year(2015)).unwrap();
        let line = "@scope (.card) { li:has(a) { color: red } } /* baselinefit:ignore scope */";
        let ids: Vec<&str> = m
            .scan("a.css", line)
            .iter()
            .map(|h| cat.features[h.feature_index].id.as_str())
            .collect();
        assert_eq!(ids, ["has"]);

        let both = "@scope (.card) { li:has(a) { } } /* baselinefit:ignore scope, has */";
        assert!(m.scan("a.css", both).is_empty());
    }

    #[test]
    fn an_unrecognised_word_after_the_marker_falls_back_to_suppress_all() {
        let cat = catalog();
        let m = Matcher::new(&cat, Target::Year(2015)).unwrap();
        for line in [
            "@scope (.card) { } /* baselinefit:ignore legacy theme */",
            "@scope (.card) { } /* baselinefit:ignore: see TICKET-12 */",
            "@scope (.card) { } // baselinefit:ignore typoed-id",
        ] {
            assert!(m.scan("a.css", line).is_empty(), "{line}");
        }
    }

    #[test]
    fn scope_globs_restrict_a_detector_to_matching_paths() {
        let cat = Catalog::from_json(
            r#"{"schema": 1,
                "dataset": {"package": "web-features", "version": "3.0.0",
                  "source_url": "https://registry.npmjs.org/x.tgz", "sha256": "a",
                  "retrieved_on": "2026-08-12", "generator": "tools/fetch-data.py"},
                "features": [{"id": "has", "name": ":has()", "description": "d",
                  "baseline": "high", "baseline_low_date": "2023-12-19",
                  "baseline_high_date": "2026-06-19",
                  "risk_action": "Use a class instead of the selector.",
                  "detectors": [{"direction": "risk", "when": "unsatisfied",
                    "kind": "selector", "text": ":has(", "what": "selector",
                    "scope": ["*.css", "src/**/*.scss"]}]}]}"#,
            "test",
        )
        .unwrap();
        let m = Matcher::new(&cat, Target::Year(2015)).unwrap();
        assert_eq!(m.scan("a.css", "li:has(a){}").len(), 1);
        assert_eq!(m.scan("deep/a.css", "li:has(a){}").len(), 1);
        assert_eq!(m.scan("src/x/y.scss", "li:has(a){}").len(), 1);
        assert_eq!(m.scan("other/y.scss", "li:has(a){}").len(), 0);
        assert_eq!(m.scan("a.js", "li:has(a){}").len(), 0);
    }

    #[test]
    fn hits_come_back_in_feature_then_direction_then_line_order() {
        let cat = catalog();
        // Container queries became Newly available in 2023, so a 2022 target
        // still treats them as a risk and both features report.
        let m = Matcher::new(&cat, Target::Year(2022)).unwrap();
        let content = "@scope (.a) {}\n@container (min-width: 1px) {}\n@scope (.b) {}\n";
        let got: Vec<(&str, usize)> = m
            .scan("a.css", content)
            .iter()
            .map(|h| (cat.features[h.feature_index].id.as_str(), h.line))
            .collect();
        // container-queries sorts before scope in the catalog, and each
        // feature's lines come back ascending.
        assert_eq!(got, [("container-queries", 2), ("scope", 1), ("scope", 3)]);
    }

    #[test]
    fn scanning_is_deterministic() {
        let cat = catalog();
        let m = Matcher::new(&cat, Target::Widely).unwrap();
        let content = "@scope (.a) {}\nli:has(b){}\n-webkit-box-shadow: 0 0 1px #000;\n";
        assert_eq!(m.scan("a.css", content), m.scan("a.css", content));
    }

    #[test]
    fn snippets_are_trimmed_and_capped_around_the_match() {
        assert_eq!(snippet("    hello    ", 5), "hello");
        let long = format!("{}NEEDLE{}", "x".repeat(400), "y".repeat(400));
        let s = snippet(&long, 401);
        assert!(
            s.chars().count() <= SNIPPET_MAX + 6,
            "{}",
            s.chars().count()
        );
        assert!(s.contains("NEEDLE"), "{s}");
        assert!(s.starts_with("..."), "{s}");
        assert!(s.ends_with("..."), "{s}");
        let tail = format!("{}NEEDLE", "x".repeat(400));
        assert!(snippet(&tail, 401).ends_with("NEEDLE"));
    }

    #[test]
    fn snippets_respect_character_boundaries() {
        let line = format!("{}@container", "あ".repeat(200));
        let s = snippet(&line, 201);
        assert!(s.contains("@container"), "{s}");
        assert!(s.chars().count() <= SNIPPET_MAX + 6);
    }

    #[test]
    fn the_builtin_snapshot_compiles_at_every_target_shape() {
        let cat = Catalog::builtin();
        for target in [Target::Widely, Target::Newly, Target::Year(2020)] {
            let m = Matcher::new(&cat, target).unwrap();
            // Every target silences some detectors -- that is the whole point
            // -- and leaves plenty live.
            assert!(m.live_detectors() > 0, "{target:?}");
            assert!(m.live_detectors() < cat.detector_count(), "{target:?}");
        }
    }
}
