//! Walking a repository and feeding files to the [`Matcher`].
//!
//! This is the only module that touches the filesystem. The walk is
//! gitignore-aware (`.gitignore`, `.ignore`, `.git/info/exclude`, the global
//! gitignore and any parent's), the same behavior sunsetscan and agentsdoc
//! have, so a scan sees what the repository actually tracks rather than what
//! happens to be on disk.

use std::path::{Path, PathBuf};

use ignore::overrides::OverrideBuilder;
use ignore::WalkBuilder;

use crate::matcher::{FileHits, Matcher};
use crate::model::ScanStats;

/// Files larger than this are skipped. Hand-written stylesheets are not this
/// big; the things that are -- minified bundles, source maps, checked-in
/// vendor builds -- produce noise rather than decisions.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// How much of a file is inspected before deciding it is binary.
const BINARY_SNIFF_BYTES: usize = 8192;

/// Extensions scanned by default. Anything else needs `--all-files`.
///
/// The list is front-end shaped: stylesheets and their preprocessors, template
/// and component formats that carry inline `<style>`, scripts, and the
/// manifests that name a polyfill package.
pub const SOURCE_EXTENSIONS: &[&str] = &[
    "astro",
    "cjs",
    "cshtml",
    "css",
    "ejs",
    "erb",
    "handlebars",
    "hbs",
    "htm",
    "html",
    "jinja",
    "js",
    "json",
    "json5",
    "jsonc",
    "jsx",
    "less",
    "liquid",
    "md",
    "mdx",
    "mjs",
    "mts",
    "njk",
    "pcss",
    "php",
    "postcss",
    "pug",
    "sass",
    "scss",
    "styl",
    "stylus",
    "svelte",
    "ts",
    "tsx",
    "twig",
    "vue",
];

/// Traversal settings.
#[derive(Debug, Clone, Default)]
pub struct ScanOptions {
    /// Scan every text file, not just the default source extensions.
    pub all_files: bool,
    /// gitignore-syntax globs whose matches are skipped.
    pub exclude: Vec<String>,
}

/// True when a repository-relative path is one of the default source kinds.
pub fn looks_like_source(rel_path: &str) -> bool {
    let name = rel_path.rsplit('/').next().unwrap_or(rel_path);
    match name.rsplit_once('.') {
        // A leading dot is the whole name (".gitignore"), not an extension.
        Some((stem, ext)) if !stem.is_empty() => SOURCE_EXTENSIONS.contains(&ext),
        _ => false,
    }
}

/// True when `bytes` look like a binary file.
fn looks_binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(BINARY_SNIFF_BYTES)].contains(&0)
}

/// The nearest ancestor of `root` (including `root` itself) that holds a
/// `.git`, i.e. the repository `root` belongs to.
fn enclosing_repo(root: &Path) -> Option<PathBuf> {
    let start = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut dir: Option<&Path> = Some(&start);
    while let Some(d) = dir {
        if d.join(".git").exists() {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    None
}

/// Directories between `repo` and `root`, outermost first, excluding `root`
/// itself (whose ignore files the walk reads on its own).
///
/// Used to honor a repository's own `.gitignore` when only a subdirectory is
/// being scanned, without letting the search escape the repository.
fn dirs_between(repo: &Path, root: &Path) -> Vec<PathBuf> {
    if repo == root {
        return Vec::new();
    }
    let mut dirs = Vec::new();
    let mut dir = root.parent();
    while let Some(d) = dir {
        dirs.push(d.to_path_buf());
        if d == repo {
            break;
        }
        dir = d.parent();
    }
    dirs.reverse();
    dirs
}

/// Ignore-file names read from directories above the scan root.
const IGNORE_FILENAMES: [&str; 3] = [".gitignore", ".ignore", ".baselinefitignore"];

/// Teach `walker` about the ignore files between the enclosing repository and
/// `root`, and decide whether the crate's own parent ascent is wanted.
///
/// The `ignore` crate's `parents(true)` climbs all the way to the filesystem
/// root, which is wrong once `root` is inside a repository: a `.gitignore`
/// above a repository does not apply to it, and a workspace directory holding
/// a bare `*` would otherwise make every repository inside it scan as
/// completely empty. So when `root` belongs to a repository the ascent is
/// switched off and the repository's own ignore files are added explicitly,
/// which keeps `baselinefit src/styles` honoring the checkout's `.gitignore`
/// while stopping at the repository boundary. Outside a repository there is no
/// boundary to respect, and the ordinary ascent applies.
fn apply_parent_ignores(walker: &mut WalkBuilder, root: &Path) {
    let Some(repo) = enclosing_repo(root) else {
        walker.parents(true);
        return;
    };
    walker.parents(false);
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    for dir in dirs_between(&repo, &canonical) {
        for name in IGNORE_FILENAMES {
            let path = dir.join(name);
            if path.is_file() {
                walker.add_ignore(path);
            }
        }
    }
}

/// Walk `root` and run `matcher` over every file that survives the filters.
///
/// Returns the per-file hits (in walk order; [`crate::model::Report::build`]
/// sorts them) and the counts of what was skipped and why.
pub fn scan_tree(
    root: &Path,
    opts: &ScanOptions,
    matcher: &Matcher,
) -> Result<(Vec<FileHits>, ScanStats), String> {
    let mut overrides = OverrideBuilder::new(root);
    for glob in &opts.exclude {
        // Override treats a bare glob as an allowlist, so an exclusion is the
        // negated form. This gives --exclude full gitignore glob syntax and
        // lets the walker prune whole directories.
        overrides
            .add(&format!("!{glob}"))
            .map_err(|e| format!("invalid --exclude glob '{glob}': {e}"))?;
    }
    let overrides = overrides
        .build()
        .map_err(|e| format!("invalid --exclude globs: {e}"))?;

    let mut walker = WalkBuilder::new(root);
    walker
        // Dotfiles are scanned: .stylelintrc and dot-prefixed config carry
        // package names.
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .ignore(true)
        // Honor .gitignore even when the target is not inside a git repo.
        .require_git(false)
        .add_custom_ignore_filename(".baselinefitignore")
        .overrides(overrides)
        .filter_entry(|e| e.file_name() != ".git");
    // See `apply_parent_ignores`; this replaces the crate's unbounded ascent.
    apply_parent_ignores(&mut walker, root);

    let mut out = Vec::new();
    let mut stats = ScanStats::default();

    for result in walker.build() {
        let dent = match result {
            Ok(d) => d,
            // A directory we cannot read is not a scan failure; note it and
            // carry on, so one unreadable path does not lose the whole report.
            Err(_) => {
                stats.skipped_unreadable += 1;
                continue;
            }
        };
        if !dent.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = dent.path();
        let rel = match path.strip_prefix(root) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => path.to_string_lossy().replace('\\', "/"),
        };

        if !opts.all_files && !looks_like_source(&rel) {
            stats.skipped_extension += 1;
            continue;
        }
        match dent.metadata() {
            Ok(md) if md.len() > MAX_FILE_BYTES => {
                stats.skipped_large += 1;
                continue;
            }
            Ok(_) => {}
            Err(_) => {
                stats.skipped_unreadable += 1;
                continue;
            }
        }

        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => {
                stats.skipped_unreadable += 1;
                continue;
            }
        };
        if looks_binary(&bytes) {
            stats.skipped_binary += 1;
            continue;
        }
        let Ok(text) = String::from_utf8(bytes) else {
            stats.skipped_binary += 1;
            continue;
        };

        stats.scanned += 1;
        let hits = matcher.scan(&rel, &text);
        if !hits.is_empty() {
            out.push((rel, hits));
        }
    }

    Ok((out, stats))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_detection_covers_the_front_end_extensions() {
        for yes in [
            "a.css",
            "src/styles/main.scss",
            "components/Card.vue",
            "app/page.tsx",
            "index.html",
            "package.json",
            "docs/notes.md",
            "themes/site.less",
        ] {
            assert!(looks_like_source(yes), "{yes} should be scanned");
        }
        for no in [
            "a.png",
            "fonts/x.woff2",
            "logo.svg",
            ".gitignore",
            "LICENSE",
            "bin/tool",
            "archive.tar.gz",
            "main.rs",
        ] {
            assert!(!looks_like_source(no), "{no} should be skipped");
        }
    }

    #[test]
    fn binary_sniffing_keys_on_nul_bytes() {
        assert!(!looks_binary(b".a { color: red }\n"));
        assert!(looks_binary(b"\x7fELF\x02\x01\x01\x00"));
        assert!(!looks_binary(b""));
        // A NUL past the sniff window does not count.
        let mut late = vec![b'a'; BINARY_SNIFF_BYTES + 10];
        late[BINARY_SNIFF_BYTES + 5] = 0;
        assert!(!looks_binary(&late));
    }

    #[test]
    fn the_extension_list_is_sorted_and_free_of_duplicates() {
        // Sorted so that adding one stays a clean diff.
        let mut sorted = SOURCE_EXTENSIONS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.as_slice(), SOURCE_EXTENSIONS);
    }
}
