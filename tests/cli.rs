//! End-to-end tests: build a project in a temporary directory, run the real
//! binary over it, and assert on what it printed and what it exited with.
//!
//! The behavior these pin down is the behavior the README promises. The most
//! important one is `the_target_flips_the_same_project_between_risk_and_cleanup`:
//! one input, two `--target` values, two opposite verdicts.

use std::path::{Path, PathBuf};
use std::process::Command;

use baselinefit::json::{self, Value};
use tempfile::TempDir;

/// A project tree in a temporary directory.
struct Project {
    dir: TempDir,
}

/// One completed run of the binary.
struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    fn ok(&self) -> &Run {
        assert_eq!(
            self.code, 0,
            "expected exit 0\nstdout:\n{}\nstderr:\n{}",
            self.stdout, self.stderr
        );
        self
    }

    fn failed(&self) -> &Run {
        assert_eq!(
            self.code, 1,
            "expected exit 1\nstdout:\n{}\nstderr:\n{}",
            self.stdout, self.stderr
        );
        self
    }

    fn has(&self, needle: &str) -> &Run {
        assert!(
            self.stdout.contains(needle),
            "expected {needle:?} in:\n{}",
            self.stdout
        );
        self
    }

    fn lacks(&self, needle: &str) -> &Run {
        assert!(
            !self.stdout.contains(needle),
            "did not expect {needle:?} in:\n{}",
            self.stdout
        );
        self
    }

    fn json(&self) -> Value {
        json::parse(&self.stdout).unwrap_or_else(|e| panic!("{e}\n{}", self.stdout))
    }
}

impl Project {
    fn new() -> Project {
        Project {
            dir: TempDir::new().expect("a temporary directory"),
        }
    }

    fn write(&self, rel: &str, contents: &str) -> &Project {
        let path = self.dir.path().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("a parent directory");
        }
        std::fs::write(&path, contents).expect("a written file");
        self
    }

    fn write_bytes(&self, rel: &str, contents: &[u8]) -> &Project {
        let path = self.dir.path().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("a parent directory");
        }
        std::fs::write(&path, contents).expect("a written file");
        self
    }

    fn run(&self, args: &[&str]) -> Run {
        run_in(self.dir.path(), args)
    }
}

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_baselinefit"))
}

fn run_in(path: &Path, args: &[&str]) -> Run {
    let output = Command::new(binary())
        .arg(path)
        .arg("--no-color")
        .args(args)
        // Keep the run independent of whoever's shell is invoking the tests.
        .env_remove("NO_COLOR")
        .output()
        .expect("the binary runs");
    Run {
        code: output.status.code().expect("a normal exit"),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// A stylesheet that is modern enough to worry about at a strict target.
const MODERN_CSS: &str = "\
.card {
  container-type: inline-size;
}

@container (min-width: 30em) {
  .card__title { font-size: 2rem; }
}

.card:has(> img) {
  padding: 0;
}
";

// ---------------------------------------------------------------------------
// The headline behavior
// ---------------------------------------------------------------------------

#[test]
fn the_target_flips_the_same_project_between_risk_and_cleanup() {
    let p = Project::new();
    p.write("styles/main.css", MODERN_CSS);

    // Container queries became Newly available 2023-02-14 and Widely
    // 2025-08-14; :has() became Newly 2023-12-19 and Widely 2026-06-19. A
    // 2022 target predates both, so both are risks and the run fails.
    let strict = p.run(&["--target", "2022"]);
    strict.failed();
    strict.has("RISK --");
    strict.has("container-queries");
    strict.has("has");

    // The default target is Widely available, which both have reached.
    let relaxed = p.run(&[]);
    relaxed.ok();
    relaxed.has("Nothing to report");
    relaxed.lacks("RISK --");
}

#[test]
fn a_prefix_pair_becomes_removable_only_once_the_target_allows_it() {
    let p = Project::new();
    p.write(
        "styles/main.css",
        ".panel {\n  -webkit-backdrop-filter: blur(6px);\n  backdrop-filter: blur(6px);\n}\n",
    );

    // backdrop-filter is Newly available (2024-09-16), not Widely. At the
    // default target the unprefixed property is the risk and the prefix is
    // still doing work, so it must not be offered for deletion.
    let widely = p.run(&[]);
    widely.failed();
    widely.has("RISK");
    widely.has("backdrop-filter: blur(6px)");
    widely.lacks("REMOVABLE --");

    // Accept Newly available and the same two lines swap meaning.
    let newly = p.run(&["--target", "newly"]);
    newly.ok();
    newly.has("REMOVABLE AT YOUR TARGET");
    newly.has("-webkit-backdrop-filter");
    newly.lacks("RISK --");
}

#[test]
fn a_widely_available_prefix_is_removable_without_qualification() {
    let p = Project::new();
    p.write(
        "styles/main.css",
        ".card {\n  -webkit-box-shadow: 0 1px 2px #0003;\n  box-shadow: 0 1px 2px #0003;\n}\n",
    );
    let run = p.run(&[]);
    run.ok();
    run.has("REMOVABLE -- Baseline Widely available");
    run.has("box-shadow");
    run.has("matched -webkit-box-shadow [property] -- vendor prefix");
}

#[test]
fn a_dead_supports_guard_is_reported_and_a_live_one_is_not() {
    let p = Project::new();
    p.write(
        "styles/main.css",
        "@supports (display: grid) {\n  .layout { display: grid; }\n}\n",
    );

    let now = p.run(&[]);
    now.ok();
    now.has("dead @supports guard");
    now.has("grid");

    // Target a year before grid was Baseline and the guard is load bearing.
    let then = p.run(&["--target", "2016"]);
    then.ok();
    then.lacks("dead @supports guard");
}

#[test]
fn a_polyfill_import_is_removable_once_the_feature_arrived() {
    let p = Project::new();
    p.write(
        "src/app.js",
        "import ResizeObserver from 'resize-observer-polyfill';\nexport default ResizeObserver;\n",
    );
    p.write(
        "package.json",
        "{\n  \"dependencies\": {\n    \"resize-observer-polyfill\": \"^1.5.1\"\n  }\n}\n",
    );

    let run = p.run(&[]);
    run.ok();
    run.has("resize-observer");
    run.has("ResizeObserver polyfill");
    run.has("src/app.js");
    run.has("package.json");
    run.has("Remove resize-observer-polyfill");
}

#[test]
fn a_javascript_api_is_a_risk_at_a_conservative_target() {
    let p = Project::new();
    p.write(
        "src/app.js",
        "const worker = new SharedWorker('/w.js');\nworker.port.start();\n",
    );
    let run = p.run(&[]);
    run.failed();
    run.has("shared-workers");
    run.has("matched SharedWorker [ident]");
}

#[test]
fn a_forward_pointer_is_information_and_never_a_deletion() {
    let p = Project::new();
    p.write(
        "src/tip.js",
        "import {computePosition} from '@floating-ui/dom';\nexport default computePosition;\n",
    );
    let run = p.run(&[]);
    run.ok();
    run.has("INFO -- pointers only, nothing to delete yet");
    run.has("Do not remove the JavaScript positioner yet");
    run.lacks("REMOVABLE --");
}

#[test]
fn a_clean_project_says_so_and_exits_zero() {
    let p = Project::new();
    p.write("styles/main.css", ".a { color: red; }\n");
    p.write("src/app.js", "export const x = 1;\n");
    let run = p.run(&[]);
    run.ok();
    run.has("Nothing to report");
    run.has("clean");
    // Even a clean run must say where its Baseline facts came from.
    run.has("Baseline data: web-features@");
}

// ---------------------------------------------------------------------------
// Filtering, suppression and the walk
// ---------------------------------------------------------------------------

#[test]
fn an_inline_marker_suppresses_a_line() {
    let p = Project::new();
    p.write(
        "styles/main.css",
        ".card:has(> img) { padding: 0 } /* baselinefit:ignore */\n@scope (.card) { a { color: red } }\n",
    );
    let run = p.run(&["--target", "2022"]);
    run.failed();
    run.lacks(":has(");
    run.has("@scope");
}

#[test]
fn a_scoped_marker_suppresses_only_the_feature_it_names() {
    let p = Project::new();
    p.write(
        "styles/main.css",
        ".card:has(> img) { text-wrap: balance } /* baselinefit:ignore has */\n",
    );
    let run = p.run(&["--target", "2022"]);
    run.failed();
    run.lacks("matched :has(");
    run.has("text-wrap-balance");
}

#[test]
fn exclude_prunes_paths_by_glob() {
    let p = Project::new();
    p.write("src/app.css", MODERN_CSS);
    p.write("vendor/bootstrap.css", MODERN_CSS);

    let all = p.run(&["--target", "2022"]);
    all.failed();
    all.has("vendor/bootstrap.css");

    let pruned = p.run(&["--target", "2022", "--exclude", "vendor/**"]);
    pruned.failed();
    pruned.lacks("vendor/bootstrap.css");
    pruned.has("src/app.css");
}

#[test]
fn the_walk_honors_gitignore_and_skips_binary_and_oversized_files() {
    let p = Project::new();
    p.write(".gitignore", "dist/\n");
    p.write("src/app.css", "@scope (.card) { a { color: red } }\n");
    p.write("dist/bundle.css", "@scope (.card) { a { color: red } }\n");
    // A NUL byte makes it binary as far as the scanner is concerned.
    p.write_bytes("src/blob.css", b"@scope (.a) {}\0\0\0");
    // Comfortably over the 2 MiB cap.
    let big = format!("@scope (.b) {{}}\n{}", "/* filler */\n".repeat(200_000));
    p.write("src/huge.css", &big);

    let run = p.run(&["--target", "2022"]);
    run.failed();
    run.has("src/app.css");
    run.lacks("dist/bundle.css");
    run.lacks("src/blob.css");
    run.lacks("src/huge.css");
    run.has("1 binary");
    run.has("1 too large");
}

#[test]
fn only_and_skip_narrow_the_catalog_and_reject_typos() {
    let p = Project::new();
    p.write(
        "styles/main.css",
        "@scope (.card) { a { color: red } }\n.card:has(a) { color: blue }\n",
    );

    let only = p.run(&["--target", "2022", "--only", "scope"]);
    only.failed();
    only.has("@scope");
    only.lacks("matched :has(");

    let skipped = p.run(&["--target", "2022", "--skip", "scope"]);
    skipped.failed();
    skipped.lacks("matched @scope");
    skipped.has(":has(");

    let typo = p.run(&["--only", "no-such-feature"]);
    assert_eq!(typo.code, 2, "{}", typo.stderr);
    assert!(
        typo.stderr.contains("unknown feature id"),
        "{}",
        typo.stderr
    );
    assert!(typo.stderr.contains("--list"), "{}", typo.stderr);
}

#[test]
fn direction_filters_the_two_halves_of_the_report() {
    let p = Project::new();
    p.write(
        "styles/main.css",
        "@scope (.card) { a { color: red } }\n.card { -webkit-box-shadow: 0 0 1px #000; }\n",
    );

    let both = p.run(&[]);
    both.has("RISK --");
    both.has("REMOVABLE --");

    let risk = p.run(&["--direction", "risk"]);
    risk.has("RISK --");
    risk.lacks("REMOVABLE --");

    let removable = p.run(&["--direction", "removable"]);
    removable.ok();
    removable.has("REMOVABLE --");
    removable.lacks("RISK --");
}

// ---------------------------------------------------------------------------
// Targets
// ---------------------------------------------------------------------------

#[test]
fn browserslist_is_read_from_package_json_and_from_the_rc_file() {
    let p = Project::new();
    p.write("styles/main.css", MODERN_CSS);
    p.write(
        "package.json",
        "{\n  \"browserslist\": [\"baseline 2022\"]\n}\n",
    );

    let from_package = p.run(&["--target", "browserslist"]);
    from_package.failed();
    from_package.has("[from package.json: \"baseline 2022\"]");
    from_package.has("Baseline 2022");

    let q = Project::new();
    q.write("styles/main.css", MODERN_CSS);
    q.write(".browserslistrc", "# comment\nbaseline widely available\n");
    let from_rc = q.run(&["--target", "browserslist"]);
    from_rc.ok();
    from_rc.has("[from .browserslistrc: \"baseline widely available\"]");
}

#[test]
fn an_unmappable_browserslist_query_is_refused_rather_than_guessed_at() {
    let p = Project::new();
    p.write("styles/main.css", MODERN_CSS);
    p.write(
        "package.json",
        "{\n  \"browserslist\": [\"> 0.5%\", \"not dead\"]\n}\n",
    );
    let run = p.run(&["--target", "browserslist"]);
    assert_eq!(run.code, 2, "{}", run.stdout);
    assert!(run.stderr.contains("combines 2 queries"), "{}", run.stderr);
    assert!(run.stderr.contains("--target"), "{}", run.stderr);

    let q = Project::new();
    q.write("styles/main.css", MODERN_CSS);
    q.write(
        "package.json",
        "{\n  \"browserslist\": \"last 2 versions\"\n}\n",
    );
    let run = q.run(&["--target", "browserslist"]);
    assert_eq!(run.code, 2);
    assert!(
        run.stderr.contains("unsupported browserslist query"),
        "{}",
        run.stderr
    );
}

#[test]
fn browserslist_with_no_configuration_explains_itself() {
    let p = Project::new();
    p.write("styles/main.css", MODERN_CSS);
    let run = p.run(&["--target", "browserslist"]);
    assert_eq!(run.code, 2);
    assert!(
        run.stderr.contains("no browserslist configuration"),
        "{}",
        run.stderr
    );
}

#[test]
fn a_nonsense_target_is_a_usage_error() {
    let p = Project::new();
    p.write("styles/main.css", MODERN_CSS);
    for bad in ["sometimes", "2014", "20260"] {
        let run = p.run(&["--target", bad]);
        assert_eq!(run.code, 2, "{bad}: {}", run.stdout);
    }
}

// ---------------------------------------------------------------------------
// Exit codes
// ---------------------------------------------------------------------------

#[test]
fn exit_codes_follow_the_documented_policy() {
    let p = Project::new();
    // Cleanup only: no risk at the default target.
    p.write(
        "styles/main.css",
        ".card { -webkit-box-shadow: 0 0 1px #000; }\n",
    );
    p.run(&[]).ok();
    p.run(&["--fail-on", "none"]).ok();
    p.run(&["--strict"]).failed();

    // Now add something unsafe for the target.
    p.write("styles/risk.css", "@scope (.card) { a { color: red } }\n");
    p.run(&[]).failed();
    p.run(&["--fail-on", "risk"]).failed();
    p.run(&["--fail-on", "none"]).ok();
    p.run(&["--strict"]).failed();
}

#[test]
fn a_missing_path_is_an_io_error_not_a_finding() {
    let run = run_in(Path::new("/no/such/directory/anywhere"), &[]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("not a directory"), "{}", run.stderr);
}

#[test]
fn strict_and_fail_on_cannot_be_combined() {
    let p = Project::new();
    p.write("styles/main.css", ".a{}\n");
    let run = p.run(&["--strict", "--fail-on", "risk"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("cannot be used with"), "{}", run.stderr);
}

// ---------------------------------------------------------------------------
// Output modes
// ---------------------------------------------------------------------------

#[test]
fn the_list_view_prints_the_snapshot_with_its_provenance() {
    let p = Project::new();
    let run = p.run(&["--list"]);
    run.ok();
    run.has("baselinefit catalog:");
    run.has("container-queries");
    run.has("anchor-positioning");
    run.has("Baseline data: web-features@3.45.0");
    run.has("retrieved 2026-11-11");
    run.has("sha256 f1b743f337b877178e5ddc264151e9e6627af26096ba96cddda8ddd7e6907e25");
    // The target changes the "reached" column, not the catalog.
    let strict = p.run(&["--list", "--target", "2015"]);
    strict.ok();
    assert!(strict.stdout.matches("not yet").count() > run.stdout.matches("not yet").count());
}

#[test]
fn the_list_json_is_parseable_and_carries_the_dataset_block() {
    let p = Project::new();
    let run = p.run(&["--list", "--json"]);
    run.ok();
    let doc = run.json();
    assert_eq!(doc.get("tool").unwrap().as_str(), Some("baselinefit"));
    let dataset = doc.get("dataset").unwrap();
    assert_eq!(
        dataset.get("package").unwrap().as_str(),
        Some("web-features")
    );
    assert_eq!(dataset.get("version").unwrap().as_str(), Some("3.45.0"));
    assert_eq!(
        dataset.get("retrieved_on").unwrap().as_str(),
        Some("2026-11-11")
    );
    let features = doc.get("features").unwrap().as_array().unwrap();
    assert!(features.len() >= 40);
}

#[test]
fn the_json_report_has_a_stable_documented_shape() {
    let p = Project::new();
    p.write("styles/main.css", MODERN_CSS);
    let run = p.run(&["--target", "2022", "--json"]);
    assert_eq!(run.code, 1);
    let doc = run.json();

    assert_eq!(doc.get("tool").unwrap().as_str(), Some("baselinefit"));
    assert_eq!(doc.get("schema"), Some(&Value::Number(1.0)));
    assert!(doc.get("version").unwrap().as_str().is_some());
    assert_eq!(
        doc.get("target").unwrap().get("level").unwrap().as_str(),
        Some("2022")
    );
    assert_eq!(
        doc.get("target")
            .unwrap()
            .get("browserslist_query")
            .unwrap()
            .as_str(),
        Some("baseline 2022")
    );
    assert!(doc.get("target").unwrap().get("source").unwrap().is_null());
    assert!(doc.get("features_searched").unwrap().as_str().is_none());

    let totals = doc.get("totals").unwrap();
    for key in [
        "findings",
        "risk",
        "removable",
        "removable_at_target",
        "info",
        "files_with_findings",
        "features_matched",
    ] {
        assert!(totals.get(key).is_some(), "totals.{key} is missing");
    }
    let stats = doc.get("stats").unwrap();
    for key in [
        "scanned",
        "skipped_extension",
        "skipped_large",
        "skipped_binary",
        "skipped_unreadable",
    ] {
        assert!(stats.get(key).is_some(), "stats.{key} is missing");
    }

    let findings = doc.get("findings").unwrap().as_array().unwrap();
    assert!(!findings.is_empty());
    let first = &findings[0];
    assert_eq!(first.get("direction").unwrap().as_str(), Some("risk"));
    assert_eq!(first.get("verdict").unwrap().as_str(), Some("risk"));
    assert_eq!(first.get("reaches_target"), Some(&Value::Bool(false)));
    assert!(first.get("action").unwrap().as_str().is_some());

    let feature = first.get("feature").unwrap();
    for key in [
        "id",
        "name",
        "description",
        "baseline",
        "baseline_status",
        "baseline_low_date",
        "baseline_high_date",
        "spec",
        "caniuse",
    ] {
        assert!(feature.get(key).is_some(), "feature.{key} is missing");
    }

    let files = first.get("files").unwrap().as_array().unwrap();
    let lines = files[0].get("lines").unwrap().as_array().unwrap();
    let detectors = lines[0].get("detectors").unwrap().as_array().unwrap();
    for key in ["text", "kind", "what", "column"] {
        assert!(detectors[0].get(key).is_some(), "detector.{key} is missing");
    }
    // JSON is never colored, whatever the terminal is.
    assert!(!run.stdout.contains('\x1b'));
}

#[test]
fn output_is_byte_for_byte_deterministic() {
    let p = Project::new();
    p.write("styles/a.css", MODERN_CSS);
    p.write("styles/b.css", MODERN_CSS);
    p.write("src/app.js", "new SharedWorker('/w.js');\n");
    p.write(
        "package.json",
        "{\"dependencies\": {\"smoothscroll-polyfill\": \"^0.4.4\"}}\n",
    );

    let first = p.run(&["--target", "2022"]);
    for _ in 0..3 {
        let again = p.run(&["--target", "2022"]);
        assert_eq!(first.stdout, again.stdout);
    }
    let json_first = p.run(&["--target", "2022", "--json"]).stdout;
    let json_again = p.run(&["--target", "2022", "--json"]).stdout;
    assert_eq!(json_first, json_again);

    // Findings are ordered by verdict, then baseline, then id; files inside a
    // group are ordered by path.
    let a = first.stdout.find("styles/a.css").expect("a.css");
    let b = first.stdout.find("styles/b.css").expect("b.css");
    assert!(a < b, "files must be path-ordered");
    let risk = first.stdout.find("RISK --").expect("a risk section");
    let removable = first
        .stdout
        .find("REMOVABLE --")
        .expect("a removable section");
    assert!(risk < removable, "risk comes before cleanup");
}

#[test]
fn quiet_prints_one_line_to_stdout_and_the_framing_to_stderr() {
    let p = Project::new();
    p.write("styles/main.css", MODERN_CSS);
    let run = p.run(&["--target", "2022", "--quiet"]);
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout.lines().count(), 1, "{}", run.stdout);
    assert!(run.stdout.contains("risk"), "{}", run.stdout);
    assert!(run.stderr.contains("web-features@"), "{}", run.stderr);
}

#[test]
fn color_is_off_for_a_pipe_and_opt_out_is_always_honored() {
    let p = Project::new();
    p.write("styles/main.css", MODERN_CSS);
    // Captured output is not a terminal, so no escapes should appear at all.
    assert!(!p.run(&["--target", "2022"]).stdout.contains('\x1b'));
}

#[test]
fn help_and_version_document_the_contract() {
    let output = Command::new(binary())
        .arg("--help")
        .output()
        .expect("--help runs");
    let help = String::from_utf8_lossy(&output.stdout);
    for needle in [
        "--target",
        "--direction",
        "--fail-on",
        "--only",
        "--skip",
        "--exclude",
        "--list",
        "--strict",
        "EXIT CODES:",
        "baselinefit:ignore",
        "textual, not semantic",
    ] {
        assert!(help.contains(needle), "missing {needle:?} in --help");
    }

    let output = Command::new(binary())
        .arg("--version")
        .output()
        .expect("--version runs");
    let version = String::from_utf8_lossy(&output.stdout);
    assert!(version.contains("baselinefit"), "{version}");
}

// ---------------------------------------------------------------------------
// The bundled demo site
// ---------------------------------------------------------------------------

fn demo_site() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/demo-site")
}

#[test]
fn the_demo_site_shows_both_directions_and_flips_with_the_target() {
    let widely = run_in(&demo_site(), &["--target", "widely"]);
    widely.failed();
    widely.has("RISK --");
    widely.has("REMOVABLE -- Baseline Widely available");
    widely.has("INFO -- pointers only");
    // backdrop-filter is Newly available, so at this target it is a risk and
    // its prefix is not offered for deletion.
    widely.has("matched backdrop-filter [property]");
    widely.lacks("matched -webkit-backdrop-filter");

    // Still a failing run: anchor positioning and text-wrap: pretty are not
    // Baseline at any level yet. But everything that only needed 30 months has
    // moved out of the risk list.
    let newly = run_in(&demo_site(), &["--target", "newly"]);
    newly.failed();
    assert!(
        newly.stdout.matches("[RISK ").count() < widely.stdout.matches("[RISK ").count(),
        "a looser target must leave fewer risks"
    );
    newly.has("REMOVABLE AT YOUR TARGET");
    newly.has("matched -webkit-backdrop-filter");
    newly.lacks("matched backdrop-filter [property]");

    // The dead guards and the polyfills are cleanup at both targets.
    for run in [&widely, &newly] {
        run.has("dead @supports guard");
        run.has("resize-observer-polyfill");
        run.has("smoothscroll-polyfill");
        run.has("js-focus-visible");
        run.has("Do not remove the JavaScript positioner yet");
    }
}

#[test]
fn the_demo_site_browserslist_resolves_to_the_default_target() {
    let from_config = run_in(&demo_site(), &["--target", "browserslist", "--json"]);
    let doc = from_config.json();
    let target = doc.get("target").unwrap();
    assert_eq!(target.get("level").unwrap().as_str(), Some("widely"));
    assert_eq!(
        target.get("source").unwrap().get("path").unwrap().as_str(),
        Some("package.json")
    );
}
