//! Command-line entry point for baselinefit.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;

use baselinefit::catalog::{Catalog, Direction};
use baselinefit::matcher::Matcher;
use baselinefit::model::{FailPolicy, Report};
use baselinefit::report::{
    render_json, render_list_json, render_list_text, render_text, summary_line, RenderOptions,
};
use baselinefit::scan::{scan_tree, ScanOptions};
use baselinefit::target::{
    browserslist_from_package_json, parse_browserslist, BrowserslistSource, Target,
};

const AFTER_HELP: &str = "\
EXIT CODES:
  0  the run finished and nothing breached the failure policy
  1  the failure policy was breached (see --fail-on / --strict)
  2  a usage error, an unreadable path, or an unmappable browserslist config

TARGETS:
  --target widely   Baseline Widely available: interoperable for 30+ months (default)
  --target newly    Baseline Newly available: interoperable in the core browsers now
  --target 2024     Baseline 2024: became Newly available in 2024 or earlier
  --target browserslist
                    read package.json \"browserslist\" or .browserslistrc from the
                    scanned directory. Only the Baseline query forms are understood
                    ('baseline widely available', 'baseline newly available',
                    'baseline YYYY', each optionally 'with downstream'). Any other
                    query is refused rather than guessed at.

DIRECTIONS:
  risk        features used that have not reached the target
  removable   prefixes, @supports guards and polyfills the target has made redundant
  both        the default

FAILURE POLICY:
  --fail-on risk (the default) exits 1 when anything is unsafe for the target.
  --fail-on none never fails on findings. --strict exits 1 on any finding at all,
  cleanup included. Findings are printed either way; only the exit code changes.

SUPPRESSION:
  A line carrying a trailing `baselinefit:ignore` comment is skipped. Name feature
  ids after the marker (`baselinefit:ignore has, subgrid`) to suppress only those;
  anything the marker does not recognise as a feature id makes it suppress the
  whole line.

Matching is textual, not semantic. baselinefit does not parse CSS or JavaScript,
so hits in comments, strings and vendored code are reported like any other.
Baseline statuses come from an embedded web-features snapshot; run --list to see
its version and retrieval date.";

/// Audit a codebase against Web Platform Baseline, in both directions.
#[derive(Parser, Debug)]
#[command(
    name = "baselinefit",
    version,
    about,
    long_about = None,
    after_help = AFTER_HELP
)]
struct Cli {
    /// Directory to scan.
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Baseline level to judge against: widely, newly, a year, or browserslist.
    #[arg(long, value_name = "LEVEL", default_value = "widely")]
    target: String,

    /// Which question to answer.
    #[arg(long, value_name = "WHICH", default_value = "both",
          value_parser = ["both", "risk", "removable"])]
    direction: String,

    /// Print the feature snapshot with Baseline statuses and dates, then exit.
    #[arg(long)]
    list: bool,

    /// Search only this feature (repeatable).
    #[arg(long, value_name = "FEATURE_ID")]
    only: Vec<String>,

    /// Skip this feature (repeatable).
    #[arg(long, value_name = "FEATURE_ID")]
    skip: Vec<String>,

    /// Exit non-zero on risk findings, or never.
    #[arg(long, value_name = "WHAT", conflicts_with = "strict",
          value_parser = ["risk", "none"])]
    fail_on: Option<String>,

    /// Exit non-zero on any finding, cleanup included.
    #[arg(long)]
    strict: bool,

    /// Skip paths matching this gitignore-syntax glob (repeatable).
    #[arg(long, value_name = "GLOB")]
    exclude: Vec<String>,

    /// Scan every text file, not just the default front-end extensions.
    #[arg(long)]
    all_files: bool,

    /// Emit the machine-readable JSON document instead of the human report.
    #[arg(long)]
    json: bool,

    /// Never emit ANSI color codes.
    #[arg(long)]
    no_color: bool,

    /// Print only the one-line summary.
    #[arg(short, long)]
    quiet: bool,
}

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("baselinefit: {err:#}");
            std::process::exit(2);
        }
    }
}

fn run() -> Result<i32> {
    let cli = Cli::parse();

    let mut catalog = Catalog::builtin();
    catalog
        .retain(&cli.only, &cli.skip)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    catalog.retain_direction(match cli.direction.as_str() {
        "risk" => Some(Direction::Risk),
        "removable" => Some(Direction::Removable),
        _ => None,
    });

    let (target, target_source) = resolve_target(&cli.target, &cli.path)?;

    // Color is opt-out via the flag, the NO_COLOR convention, or a non-tty
    // stdout; JSON is never colored.
    let color = !cli.no_color
        && !cli.json
        && std::env::var_os("NO_COLOR").is_none()
        && std::io::stdout().is_terminal();
    let opts = RenderOptions { color };

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    if cli.list {
        if cli.json {
            out.write_all(render_list_json(&catalog, target).as_bytes())?;
        } else {
            out.write_all(render_list_text(&catalog, target, opts).as_bytes())?;
        }
        out.flush()?;
        return Ok(0);
    }

    if !cli.path.is_dir() {
        anyhow::bail!("not a directory: {}", cli.path.display());
    }
    if catalog.is_empty() {
        anyhow::bail!("--only / --skip / --direction left no features to search for");
    }

    let root_name = cli
        .path
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| cli.path.display().to_string());

    let matcher = Matcher::new(&catalog, target).map_err(|e| anyhow::anyhow!("{e}"))?;
    let scan_opts = ScanOptions {
        all_files: cli.all_files,
        exclude: cli.exclude.clone(),
    };
    let (hits, stats) = scan_tree(&cli.path, &scan_opts, &matcher)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .with_context(|| format!("failed to walk {}", cli.path.display()))?;

    let report = Report::build(root_name, target, target_source, &catalog, hits, stats);

    if cli.json {
        out.write_all(render_json(&report).as_bytes())?;
    } else if cli.quiet {
        // stdout stays a single pipeable line; the framing goes to stderr so it
        // is still shown without polluting the summary.
        writeln!(
            out,
            "baselinefit: {}  {}",
            report.root_name,
            summary_line(&report, opts)
        )?;
        eprintln!(
            "Baseline data: {}@{}, retrieved {}. Matching is textual, not semantic.",
            report.dataset.package, report.dataset.version, report.dataset.retrieved_on
        );
    } else {
        out.write_all(render_text(&report, opts).as_bytes())?;
    }
    out.flush()?;

    let policy = if cli.strict {
        FailPolicy::Any
    } else {
        match cli.fail_on.as_deref() {
            Some(value) => FailPolicy::parse(value).map_err(|e| anyhow::anyhow!("{e}"))?,
            None => FailPolicy::default(),
        }
    };
    Ok(report.exit_code(policy))
}

/// Turn `--target` into a [`Target`], reading a Browserslist config when asked.
///
/// The filesystem lookup lives here rather than in the library because the
/// library is deliberately pure; `target.rs` holds the parsing and the rules.
fn resolve_target(spec: &str, root: &Path) -> Result<(Target, Option<BrowserslistSource>)> {
    if spec != "browserslist" {
        let target = Target::parse(spec).map_err(|e| anyhow::anyhow!("{e}"))?;
        return Ok((target, None));
    }

    let package_json = root.join("package.json");
    if package_json.is_file() {
        let text = std::fs::read_to_string(&package_json)
            .with_context(|| format!("cannot read {}", package_json.display()))?;
        let found = browserslist_from_package_json(&text).map_err(|e| anyhow::anyhow!("{e}"))?;
        if let Some(body) = found {
            let (target, query) = parse_browserslist(&body)
                .map_err(|e| anyhow::anyhow!("package.json \"browserslist\": {e}"))?;
            return Ok((
                target,
                Some(BrowserslistSource {
                    path: "package.json".to_string(),
                    query,
                }),
            ));
        }
    }

    let rc = root.join(".browserslistrc");
    if rc.is_file() {
        let text = std::fs::read_to_string(&rc)
            .with_context(|| format!("cannot read {}", rc.display()))?;
        let (target, query) =
            parse_browserslist(&text).map_err(|e| anyhow::anyhow!(".browserslistrc: {e}"))?;
        return Ok((
            target,
            Some(BrowserslistSource {
                path: ".browserslistrc".to_string(),
                query,
            }),
        ));
    }

    anyhow::bail!(
        "--target browserslist found no browserslist configuration in {}: no \"browserslist\" \
         key in package.json and no .browserslistrc. Pass --target widely, newly or a year.",
        root.display()
    )
}
