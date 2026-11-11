# baselinefit

Audit a codebase against Web Platform Baseline, in both directions.

Two questions, from one dataset and one `--target`:

1. **What am I using that is not safe yet?** The forward direction. Container
   queries, `:has()`, `@scope`, `field-sizing`, `SharedWorker`, anchor
   positioning -- each measured against the Baseline level you say you target.
2. **What safety nets can I finally delete?** The backward direction. The
   `-webkit-` prefixes whose unprefixed property is Widely available, the
   `@supports` guards that always pass now, the polyfills still shipping on
   every page for a browser nobody has.

Editors, linters and Browserslist all answer the first question. Nothing
answers the second, because there is nothing wrong with the code -- it is just
no longer doing anything. It accumulates quietly for years.

Offline, single binary, no network access, no configuration file required.

```
$ baselinefit ./site --target widely
[RISK     ] backdrop-filter  backdrop-filter
  baseline     Newly available (newly 2024-09-16)
  ...
  styles/main.css
    26:5  backdrop-filter: blur(6px);

$ baselinefit ./site --target newly
[AT TARGET] backdrop-filter  backdrop-filter
  baseline     Newly available (newly 2024-09-16)
  ...
  styles/main.css
    25:5  -webkit-backdrop-filter: blur(6px);
```

Same two lines of CSS. The target decides which one is the problem.

## What Baseline is

[Baseline](https://web.dev/baseline) (read 2026-08-12) is a shared way to say
how safe a web feature is:

- **Newly available** -- "The feature is supported by all of the core browsers,
  and is therefore interoperable." The core browsers are Chrome (desktop and
  Android), Edge, Firefox (desktop and Android) and Safari (macOS and iOS).
- **Widely available** -- "30 months have passed since the newly interoperable
  date. The feature can be used by most sites without worrying about support."
- **Baseline YYYY** -- the features that became Newly available during that
  calendar year. "All items that become part of Baseline Newly available in
  2025 can be referred to as Baseline 2025."

Those are the same three shapes Browserslist exposes as `baseline widely
available`, `baseline newly available` and `baseline YYYY`
([web.dev](https://web.dev/articles/use-baseline-with-browserslist), read
2026-08-12), which is why `--target` accepts exactly them and no more.

## Where the data comes from

Every Baseline status and date baselinefit prints is copied verbatim from the
published [`web-features`](https://github.com/web-platform-dx/web-features)
dataset. Nothing is written from memory.

The current snapshot is **web-features 3.45.0**, retrieved **2026-11-11** from
`https://registry.npmjs.org/web-features/-/web-features-3.45.0.tgz` (SHA-256
`f1b743f337b877178e5ddc264151e9e6627af26096ba96cddda8ddd7e6907e25`). That
provenance is stamped into `src/features.json`, embedded in the binary, and
printed under every report and every `--list`.

`tools/fetch-data.py` is the only thing allowed to change it:

```bash
python3 tools/fetch-data.py                   # latest published version
python3 tools/fetch-data.py --version 3.34.3  # pin a version
python3 tools/fetch-data.py --check           # fail if the snapshot is stale
```

It downloads the npm tarball, reads `package/data.json`, and copies `name`,
`description`, `spec`, `status.baseline`, `status.baseline_low_date` and
`status.baseline_high_date` for exactly the ids listed in
`tools/detectors.json`. An id that is not a real dataset id fails the build, so
no status, date or feature name in this tool can have been invented. Standard
library only; no `pip install`.

**It is a snapshot.** Baseline moves: a feature reaches Widely available 30
months after it became Newly available, and dates get corrected upstream. A
build from six months ago will be six months out of date and will happily tell
you so. Re-run the generator, or check the live dataset, before a decision that
matters.

## Install

```bash
cargo install --git https://github.com/printemps-tokyo/baselinefit
```

Or download a tarball from the [releases](../../releases) page (Linux x86-64,
macOS x86-64, macOS arm64; each with a `.sha256` next to it), or build from
source:

```bash
git clone https://github.com/printemps-tokyo/baselinefit
cd baselinefit
cargo build --release
./target/release/baselinefit --help
```

## Quickstart

```bash
# Both directions, against Baseline Widely available (the default).
baselinefit ./my-site

# Only the cleanup: what can I delete?
baselinefit ./my-site --direction removable

# We have decided Newly available is good enough for us.
baselinefit ./my-site --target newly

# Take the level from the project's own browserslist config.
baselinefit ./my-site --target browserslist

# In CI: fail the build when something is unsafe for the target.
baselinefit ./my-site --target browserslist --quiet

# Machine-readable.
baselinefit ./my-site --json > baseline.json

# What does it know about?
baselinefit --list
```

## Target semantics

| `--target` | means | Browserslist equivalent | a feature satisfies it when |
|---|---|---|---|
| `widely` (default) | Baseline Widely available | `baseline widely available` | `status.baseline` is `high` |
| `newly` | Baseline Newly available | `baseline newly available` | `status.baseline` is `high` or `low` |
| `2024` (any year from 2015) | Baseline 2024 | `baseline 2024` | `status.baseline_low_date` falls in 2024 or earlier |
| `browserslist` | read from the project | -- | as above, for whichever query was found |

`--target browserslist` reads `browserslist` from `package.json`, or
`.browserslistrc`, in the directory being scanned. It understands only the
three Baseline query forms above, optionally suffixed `with downstream`
(accepted and ignored: widening the browser list to Chromium forks does not
change which Baseline level is being asked for). Anything else -- `> 0.5%`,
`last 2 versions`, `defaults`, a combination of queries, per-environment
sections -- is refused with an explanation:

```
baselinefit: package.json "browserslist": unsupported browserslist query
'last 2 versions'; baselinefit understands only 'baseline widely available',
'baseline newly available' and 'baseline YYYY' (optionally with 'with
downstream'). Pass --target explicitly.
```

Mapping `> 0.5%` onto a Baseline level would need the caniuse usage data
Browserslist itself resolves against, and that changes weekly. Refusing is the
honest answer.

### What the target decides

For each feature, the target answers one question: has this feature arrived?
Everything else follows.

| the feature has... | a **use** of it is | its prefix / guard / polyfill is |
|---|---|---|
| reached the target, and is Widely available | fine, not reported | `REMOVABLE` -- delete it |
| reached the target, but is only Newly available | fine, not reported | `AT TARGET` -- redundant *for this target* |
| not reached the target | `RISK` | still load bearing, not reported |
| no Baseline status at all | `RISK` | `INFO` at most, never a deletion |

The `AT TARGET` row exists so that "safe to delete" stays an unqualified claim.
`-webkit-backdrop-filter` is redundant if you have decided Newly available is
your bar, but `backdrop-filter` is not Widely available yet, and the report says
which of the two it means.

## The honest limitation: textual matching, not a parser

baselinefit does **not** parse CSS or JavaScript. It matches literal text, line
by line, with a documented boundary rule per detector kind. Consequences, all
of them deliberate:

- A hit inside a comment, a string, a Markdown code block or a vendored
  stylesheet is reported like any other hit.
- A construct split across lines is not seen. `@supports (\n  display: grid\n)`
  is not recognised as a guard.
- Sass, Less, JSX, Vue SFCs, Twig, Blade and plain HTML all work, because none
  of them need a dedicated parser. That is the trade: a parser per dialect
  would cover less, not more.
- `--exclude`, `.baselinefitignore` and the `baselinefit:ignore` marker exist
  because of this, not in spite of it.

One thing it does understand: a feature named inside an `@supports` condition
is being feature-detected, not used unguarded. Risk detectors skip matches
inside an `@supports` prelude on the same line, so `@supports (backdrop-filter:
blur(1px))` is never reported as a risk. Once the feature reaches the target,
the same line is reported the other way, as a dead guard.

### Boundary rules

| kind | fires when |
|---|---|
| `at-rule` | the literal appears, not preceded or followed by an identifier character. `@container` matches `@container (min-width: 20em)` and never `container-type: inline-size` or `@containerize`. |
| `property` | the literal is a complete CSS identifier followed by optional spaces (and an optional closing quote) and a `:`. `anchor-name` matches `anchor-name: --a` and `"anchor-name":`, but not `--anchor-name:` or `anchor-name-x:`. |
| `property-stem` | as `property`, except the identifier may continue with a `-` segment. `-webkit-mask` matches `-webkit-mask-image:`; `mask` matches `mask:` and `mask-image:` but not `masked:` and not `-webkit-mask-image:`. |
| `declaration` | a `property` match whose value is exactly the literal's value part. `text-wrap:balance` matches `text-wrap: balance;` but not `text-wrap: balanced`. |
| `selector` | the literal appears anywhere, with no boundary. `::view-transition` is a prefix on purpose, so it also covers `::view-transition-group`. |
| `function` | the literal (ending in `(`) appears, not preceded by an identifier character. `oklch(` never fires inside `my-oklch(`. |
| `unit` | the literal directly follows a number and is not followed by an identifier character. `lh` matches `1lh`, `.5lh` and `-1lh`, but not `line-height`, `1rlh` or `--gap-1lh`. |
| `ident` | the literal is a complete JavaScript identifier or dotted path. `ResizeObserver` does not fire inside `MyResizeObserverShim`; `navigator.clipboard` matches `window.navigator.clipboard`. |
| `package` | the literal appears on a line that also contains a quote, with package-name boundaries. That covers `import`, `require`, `package.json` dependencies and `<script src>`, and ignores the same name written in prose. |
| `supports` | the literal, read as a property or a declaration, appears inside an `@supports` condition on the same line. |
| `supports-selector` | the literal appears inside a `selector(...)` call inside an `@supports` condition. |

Matching is case-sensitive throughout, because CSS property names, at-rules and
JavaScript identifiers all are.

### Which files are scanned

Gitignore-aware: `.gitignore`, `.ignore`, `.git/info/exclude`, the global
gitignore and a `.baselinefitignore` are all honored. When the scan root sits
inside a repository the search for parent ignore files stops at the repository
boundary, so a `.gitignore` above your checkout cannot silently empty a scan.

By default only front-end extensions are read: `astro cjs cshtml css ejs erb
handlebars hbs htm html jinja js json json5 jsonc jsx less liquid md mdx mjs
mts njk pcss php postcss pug sass scss styl stylus svelte ts tsx twig vue`.
Use `--all-files` for everything. Files over 2 MiB and files containing a NUL
byte in their first 8 KiB are skipped and counted in the footer.

## Suppression

Put a trailing comment on the line:

```css
.card:has(> img) { padding: 0 }   /* baselinefit:ignore */
.card:has(> img) { padding: 0 }   /* baselinefit:ignore has */
```

```js
new SharedWorker('/w.js'); // baselinefit:ignore shared-workers, popover
```

A bare marker suppresses every feature on that line. Naming feature ids
suppresses only those. If any token after the marker is not a feature id in the
snapshot -- prose, a ticket number, a typo -- the marker degrades to the bare
form and suppresses the whole line, which is what a reader of the comment
expects. Ids come from `--list`.

For whole paths use `--exclude 'vendor/**'` (repeatable, gitignore glob syntax)
or commit a `.baselinefitignore`.

## Options

```
baselinefit [PATH]

  --target <LEVEL>       widely (default) | newly | YYYY | browserslist
  --direction <WHICH>    both (default) | risk | removable
  --only <FEATURE_ID>    search only this feature (repeatable)
  --skip <FEATURE_ID>    skip this feature (repeatable)
  --exclude <GLOB>       skip matching paths (repeatable)
  --all-files            scan every text file, not just front-end extensions
  --fail-on <WHAT>       risk (default) | none
  --strict               exit non-zero on any finding, cleanup included
  --list                 print the feature snapshot and exit
  --json                 machine-readable output
  --quiet, -q            one-line summary on stdout
  --no-color             never emit ANSI escapes
  --help, --version
```

Color is also suppressed when stdout is not a terminal, when `NO_COLOR` is set,
and always in `--json`.

## Exit codes

| code | meaning |
|---|---|
| `0` | the run finished and nothing breached the failure policy |
| `1` | the failure policy was breached |
| `2` | a usage error, an unreadable path, or an unmappable browserslist config |

The failure policy is `--fail-on risk` by default: exit 1 if anything is unsafe
for the target. `--fail-on none` never fails on findings. `--strict` exits 1 on
any finding at all, including cleanup. Findings are printed in every case; only
the exit code changes. Exit code 2 always writes an explanation to stderr and
prints no report.

## JSON output

`--json` emits one object. Field names and nesting are part of the contract and
covered by tests.

```jsonc
{
  "tool": "baselinefit",
  "version": "0.1.0",
  "schema": 1,
  "root": "my-site",
  "target": {
    "level": "widely",            // widely | newly | a year as a string
    "browserslist_query": "baseline widely available",
    "description": "Baseline Widely available (interoperable for 30+ months)",
    "source": null                // or {"path": "package.json", "query": "..."}
  },
  "dataset": {
    "package": "web-features",
    "version": "3.45.0",
    "source_url": "https://registry.npmjs.org/web-features/-/web-features-3.45.0.tgz",
    "sha256": "f1b743f3...",
    "retrieved_on": "2026-11-11",
    "generator": "tools/fetch-data.py"
  },
  "features_searched": 51,
  "totals": {
    "findings": 28, "risk": 16, "removable": 10,
    "removable_at_target": 0, "info": 2,
    "files_with_findings": 4, "features_matched": 19
  },
  "stats": {
    "scanned": 4, "skipped_extension": 0, "skipped_large": 0,
    "skipped_binary": 0, "skipped_unreadable": 0
  },
  "findings": [
    {
      "feature": {
        "id": "container-queries",
        "name": "Container queries (size)",
        "description": "...",
        "baseline": "high",                 // high | low | false, as web-features spells it
        "baseline_status": "Widely available",
        "baseline_low_date": "2023-02-14",
        "baseline_high_date": "2025-08-14",
        "spec": "https://drafts.csswg.org/css-conditional-5/#container-queries",
        "caniuse": ["css-container-queries", "css-container-query-units"]
      },
      "direction": "risk",                  // risk | removable
      "verdict": "risk",                    // risk | removable | removable_at_target | info
      "reaches_target": false,
      "action": "...",
      "count": 2,
      "files": [
        {
          "path": "styles/main.css",
          "count": 2,
          "lines": [
            {
              "line": 6, "column": 1,
              "snippet": "@container (min-width: 30em) {",
              "detectors": [
                {"text": "@container", "kind": "at-rule",
                 "what": "container query at-rule", "column": 1}
              ]
            }
          ]
        }
      ]
    }
  ],
  "caveat": "..."
}
```

`--list --json` emits the whole snapshot instead: the same `tool` / `version` /
`schema` / `target` / `dataset` header, then a `features` array where each entry
carries the same `feature` fields plus `reaches_target`, `risk_action`,
`removable_action` and its `detectors`.

## Feature catalog

51 features from web-features 3.45.0. Statuses and dates below are as of the
snapshot; run `baselinefit --list` for the authoritative copy in your build.

| feature id | Baseline | since | risk detectors | removable detectors |
|---|---|---|---|---|
| `aborting` | Widely | 2021-09-25 | - | `abortcontroller-polyfill` |
| `anchor-positioning` | Limited | - | `anchor-name`, `position-anchor`, `position-area`, `position-try-fallbacks`, `@position-try`, `anchor-size(` | `@floating-ui/`, `@popperjs/core`, `popper.js` |
| `animations-css` | Widely | 2018-03-30 | - | `-webkit-animation`, `-moz-animation`, `@-webkit-keyframes`, `@-moz-keyframes` |
| `appearance` | Widely | 2024-09-14 | - | `-webkit-appearance`, `-moz-appearance` |
| `aspect-ratio` | Widely | 2024-03-20 | - | `aspect-ratio` |
| `async-clipboard` | Newly | 2024-06-11 | `navigator.clipboard`, `ClipboardItem` | - |
| `backdrop-filter` | Newly | 2024-09-16 | `backdrop-filter` | `-webkit-backdrop-filter`, `backdrop-filter` |
| `border-radius` | Widely | 2018-01-29 | - | `-webkit-border-radius`, `-moz-border-radius` |
| `box-shadow` | Widely | 2018-01-29 | - | `-webkit-box-shadow`, `-moz-box-shadow` |
| `cascade-layers` | Widely | 2024-09-14 | `@layer`, `layer(` | - |
| `clip-path` | Widely | 2023-07-21 | - | `-webkit-clip-path` |
| `color-mix` | Widely | 2025-11-09 | `color-mix(` | - |
| `container-queries` | Widely | 2025-08-14 | `@container`, `container-type`, `container-name`, `cqw`, `cqh`, `cqi`, `cqb`, `cqmin`, `cqmax` | `container-type`, `container-query-polyfill` |
| `content-visibility` | Newly | 2025-09-15 | `content-visibility` | - |
| `custom-properties` | Widely | 2019-10-05 | - | `css-vars-ponyfill` |
| `dialog` | Widely | 2024-09-14 | - | `dialog-polyfill` |
| `fetch` | Widely | 2019-09-27 | - | `whatwg-fetch`, `isomorphic-fetch` |
| `field-sizing` | Newly | 2026-06-16 | `field-sizing` | - |
| `filter` | Widely | 2019-03-07 | - | `-webkit-filter` |
| `flexbox` | Widely | 2018-03-30 | - | `-webkit-flex`, `-webkit-align`, `-webkit-justify-content`, `-webkit-order`, `-ms-flex`, `display:-webkit-flex`, `display:-ms-flexbox`, `display:flex` |
| `flexbox-gap` | Widely | 2023-10-26 | - | `gap` |
| `focus-visible` | Widely | 2024-09-14 | - | `focus-visible.js`, `focus-visible.min.js`, `js-focus-visible`, `:focus-visible` |
| `grid` | Widely | 2020-04-17 | - | `display:grid` |
| `has` | Widely | 2026-06-19 | `:has(` | `:has(`, `css-has-pseudo` |
| `hyphens` | Widely | 2026-03-18 | - | `-webkit-hyphens`, `-ms-hyphens` |
| `intersection-observer` | Widely | 2021-09-25 | - | `intersection-observer` |
| `lh` | Widely | 2026-05-21 | `lh` | - |
| `light-dark` | Newly | 2024-05-13 | `light-dark(` | - |
| `masks` | Widely | 2026-06-07 | `mask` | `-webkit-mask` |
| `object-fit` | Widely | 2022-07-15 | - | `object-fit`, `object-fit-images`, `objectFitPolyfill` |
| `oklab` | Widely | 2025-11-09 | `oklch(`, `oklab(` | - |
| `popover` | Newly | 2025-01-27 | `showPopover`, `hidePopover`, `togglePopover`, `popovertarget` | `@oddbird/popover-polyfill` |
| `resize-observer` | Widely | 2023-01-28 | `ResizeObserver` | `resize-observer-polyfill`, `@juggle/resize-observer` |
| `rlh` | Widely | 2026-05-21 | `rlh` | - |
| `scope` | Newly | 2026-03-24 | `@scope` | - |
| `scroll-behavior` | Widely | 2024-09-14 | - | `smoothscroll-polyfill`, `scroll-behavior-polyfill` |
| `scrollbar-gutter` | Newly | 2024-12-11 | `scrollbar-gutter` | - |
| `shared-workers` | Newly | 2026-05-05 | `SharedWorker` | - |
| `starting-style` | Newly | 2024-08-06 | `@starting-style` | - |
| `sticky-positioning` | Widely | 2022-03-19 | - | `position:-webkit-sticky`, `position:sticky` |
| `structured-clone` | Widely | 2024-09-14 | `structuredClone` | `@ungap/structured-clone` |
| `subgrid` | Widely | 2026-03-15 | `grid-template-columns:subgrid`, `grid-template-rows:subgrid` | - |
| `text-wrap-balance` | Newly | 2024-05-13 | `text-wrap:balance` | - |
| `text-wrap-pretty` | Limited | - | `text-wrap:pretty` | - |
| `transforms2d` | Widely | 2018-03-30 | - | `-webkit-transform`, `-moz-transform`, `-ms-transform`, `-o-transform` |
| `transitions` | Widely | 2018-03-30 | - | `-webkit-transition`, `-moz-transition`, `-o-transition` |
| `trusted-types` | Newly | 2026-02-24 | `trustedTypes`, `TrustedHTML` | - |
| `urlpattern` | Newly | 2025-09-15 | `URLPattern` | `urlpattern-polyfill` |
| `user-select` | Limited | - | `user-select` | - |
| `view-transitions` | Newly | 2025-10-14 | `view-transition-name`, `@view-transition`, `::view-transition`, `startViewTransition` | - |
| `viewport-unit-variants` | Widely | 2025-06-05 | `dvh`, `dvw`, `svh`, `svw`, `lvh`, `lvw` | - |

`since` is the Widely-available date where there is one, otherwise the
Newly-available date. `-` means the feature is not Baseline at all.

Note the two entries that appear in both columns. `container-type` is a risk as
a declaration and a removable dead guard inside `@supports`; `:has(` likewise,
as a selector and inside `@supports selector(...)`. They cannot both fire: a
feature has either reached your target or it has not.

## A worked example

`tests/fixtures/demo-site` is a small site that mixes both directions on
purpose. Run it yourself:

```bash
baselinefit tests/fixtures/demo-site --target widely
baselinefit tests/fixtures/demo-site --target newly
```

At `--target widely`:

```
target widely  |  16 risk  |  16 risk, 10 removable, 2 info across 4 file(s); 19 of 51 feature(s) matched

RISK -- used, but not safe for your target yet
[RISK     ] anchor-positioning  Anchor positioning
[RISK     ] text-wrap-pretty  text-wrap: pretty
[RISK     ] backdrop-filter  backdrop-filter
[RISK     ] field-sizing  field-sizing
[RISK     ] popover  Popover
[RISK     ] scope  @scope
[RISK     ] shared-workers  Shared worker
[RISK     ] starting-style  @starting-style
[RISK     ] text-wrap-balance  text-wrap: balance
[RISK     ] trusted-types  Trusted types
[RISK     ] urlpattern  URLPattern
[RISK     ] view-transitions  View transitions

REMOVABLE -- Baseline Widely available, so the safety net can go
[REMOVABLE] border-radius  border-radius
[REMOVABLE] box-shadow  box-shadow
[REMOVABLE] flexbox-gap  Flexbox gap
[REMOVABLE] focus-visible  :focus-visible
[REMOVABLE] grid  Grid
[REMOVABLE] resize-observer  Resize observer
[REMOVABLE] scroll-behavior  scroll-behavior

INFO -- pointers only, nothing to delete yet
[INFO     ] anchor-positioning  Anchor positioning
```

At `--target newly`, twelve risky features become two, and
`-webkit-backdrop-filter` moves from load-bearing to redundant:

```
target newly  |  4 risk  |  4 risk, 11 removable, 2 info across 3 file(s); 10 of 51 feature(s) matched

RISK -- used, but not safe for your target yet
[RISK     ] anchor-positioning  Anchor positioning
[RISK     ] text-wrap-pretty  text-wrap: pretty

REMOVABLE -- Baseline Widely available, so the safety net can go
[REMOVABLE] border-radius  border-radius
[REMOVABLE] box-shadow  box-shadow
[REMOVABLE] flexbox-gap  Flexbox gap
[REMOVABLE] focus-visible  :focus-visible
[REMOVABLE] grid  Grid
[REMOVABLE] resize-observer  Resize observer
[REMOVABLE] scroll-behavior  scroll-behavior

REMOVABLE AT YOUR TARGET -- redundant for this target, not yet Widely available
[AT TARGET] backdrop-filter  backdrop-filter

INFO -- pointers only, nothing to delete yet
[INFO     ] anchor-positioning  Anchor positioning
```

Two features stay in the risk list at every target, because they are not
Baseline at any level: anchor positioning and `text-wrap: pretty`. That is also
why the Floating UI import stays under INFO with "do not remove the JavaScript
positioner yet" rather than being offered as a deletion. When CSS anchor
positioning does become Baseline, that INFO becomes a real removal, and the
report will say so on its own.

## Continuous integration

```yaml
- name: Baseline audit
  run: baselinefit . --target browserslist --quiet
```

Exit 1 means something in the tree is not safe for the target. Cleanup findings
never fail the build unless you ask for `--strict`. Use `--fail-on none` to
report without gating.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md), which covers refreshing the dataset
snapshot, adding a feature to `tools/detectors.json`, and the rules around
detection changes. The short version: no Baseline fact is ever written by hand,
and nothing is ever called removable unless the dataset says so.

## License

MIT. See [LICENSE](LICENSE).
