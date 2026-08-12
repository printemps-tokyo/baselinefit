# Contributing

Thanks for your interest in contributing to baselinefit.

## Development

1. Fork / clone the repository
2. Install a stable Rust toolchain (https://rustup.rs)
3. Create a branch: `git switch -c feat/your-change`
4. Make your change and verify locally:
   ```bash
   cargo fmt --all -- --check
   cargo clippy --all-targets -- -D warnings
   cargo test --all
   cargo build --release
   ```
5. Commit and open a pull request

CI pins the toolchain (see `RUST_TOOLCHAIN` in `.github/workflows/ci.yml`). If
your local clippy is older, a lint added in a newer release can still fail CI;
install the pinned version with `rustup toolchain install <version>` and run
`cargo +<version> clippy --all-targets -- -D warnings` before pushing.

## Refreshing the Baseline snapshot

`src/features.json` is generated. Never edit it by hand.

```bash
python3 tools/fetch-data.py                 # latest published web-features
python3 tools/fetch-data.py --version 3.34.3  # pin a version
python3 tools/fetch-data.py --check         # fail if the snapshot is stale
```

The generator downloads the published `web-features` npm package, reads
`package/data.json` out of the tarball, and copies `name`, `description`,
`spec`, `status.baseline`, `status.baseline_low_date` and
`status.baseline_high_date` verbatim for exactly the ids listed in
`tools/detectors.json`. It stamps the version, the tarball URL, its SHA-256 and
the retrieval date into the output. Standard library only; no `pip install`.

Two rules follow from this and they are not negotiable:

- No Baseline fact is ever written by hand. If an id is not in the dataset, or
  has become a `moved` / `split` record, the generator fails rather than
  inventing an entry. Follow the redirect and fix `tools/detectors.json`.
- The snapshot is a snapshot. Every report and every `--list` prints the
  version and the retrieval date, because a status with no date on it is a
  rumour.

Refreshing usually moves dates and can move a feature from Newly to Widely
available. That changes verdicts, so a refresh commit should say which features
moved. A few tests assert specific dates (`catalog.rs`, `target.rs`); update
them in the same commit and check the diff is only what the dataset actually
changed.

## Adding a feature to the catalog

Add an entry to `tools/detectors.json`, keyed by its **real web-features id**,
then re-run the generator. That file holds no Baseline facts at all: only the
text patterns to look for and the prose advice to print.

A pull request adding a feature should:

- use a real dataset id. The generator refuses anything else, so a typo is a
  build failure rather than a wrong report
- write both `risk_action` and `removable_action` where both directions apply.
  The advice is what makes a finding actionable; "this is not Baseline" on its
  own is not worth printing
- pick `kind` deliberately. The boundary rules are documented in the README and
  in `src/matcher.rs`, and they are the difference between finding real call
  sites and firing on a neighbour. `at-rule` for `@container`, `property` for a
  declaration name, `property-stem` for a prefix family, `declaration` for a
  specific value, `unit` for a unit, `ident` for JavaScript, `package` for an
  import, `supports` / `supports-selector` for a feature query
- never point a removable detector at a feature that is not Baseline. A
  "delete this" finding is only ever justified by the feature having reached
  the target; a forward pointer must use `"when": "always"` and say plainly
  that nothing should be removed yet. There is a test that enforces this
- add a test to `tests/cli.rs`: a hit, and a near-miss that must **not** match
- add a row to the feature table in `README.md` in the same change

## Detection changes

Matching semantics are documented in `README.md` and in the module
documentation of `src/matcher.rs`, and pinned by tests there. Changing a
boundary rule changes what every existing detector matches, so a pull request
that touches one needs to say which features it affects and add cases on both
sides of the boundary.

baselinefit deliberately does **not** parse CSS or JavaScript, and does not try
to work out whether a hit is inside a comment. That is stated plainly in the
README rather than half-solved. Proposals to add a parser should start as an
issue, because the trade -- fewer dialects covered, more code, more ways to be
subtly wrong -- is a project-level decision.

## Target semantics

`--target` accepts only the three Baseline shapes that Browserslist documents:
`widely`, `newly` and a year. Any other browserslist query is refused with an
explanation instead of being mapped onto a guess, because mapping `> 0.5%` onto
a Baseline level honestly would need the caniuse usage data that Browserslist
itself resolves against. Pull requests that add a heuristic for those queries
will not be accepted; pull requests that improve the error message will.

## Dependencies

baselinefit depends on `anyhow`, `clap` and `ignore`, and that is meant to stay
short. JSON, dates and text wrapping are local code on purpose. A pull request
adding a dependency needs to explain why a few dozen lines of local code is
worse. Dependency updates are handled by Dependabot (see
`.github/dependabot.yml`).

## Commit messages

[Conventional Commits](https://www.conventionalcommits.org/) are preferred
(`feat:`, `fix:`, `docs:`, `chore:`, `refactor:`, `test:`). English, no emoji.

## Pull requests

- Keep one PR focused on a single purpose
- CI (fmt / clippy / test / build / self-scan) must be green
- Update the README and tests when behavior changes

## Bug reports and requests

Please open an [issue](../../issues) using the templates. For a false positive
or a false negative, include the line that did or did not match, the feature id
and the `--target` you used -- that is usually enough to fix it.
