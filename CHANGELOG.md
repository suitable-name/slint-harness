# Changelog

## slint-harness 0.18.5

- Timing statistics: the new `timing` module (`record`, `record_timeout`, `time`; in the prelude)
  and `backend::wait_until_labeled`. The backend records `wait`, `settle` and `pump` by itself.
  Each scenario prints a `timing:` console line (the six labels with the most time), `report.json`
  gets `scenarios[i].timing` and a top-level `timing` (totals, parallel workers and utilisation),
  and the report files get a "Timing" section (15 slowest scenarios, time and timeouts per label,
  utilisation of a parallel run).
- `Harness::scenario_last`: a scenario that runs after every ordinary one. In a parallel run the
  coordinator dispatches it when all ordinary scenarios are done and hands it the union of the
  workers' coverage recorders (every `done` message now carries the worker's recorder), so
  `coverage::recorded_callbacks()` inside it sees the whole run.

## slint-harness 0.18.4

- Screenshots (opt-in feature `screenshots`): `Harness::renderer(Renderer::Software)` draws the
  windows with Slint's software renderer; the new `screenshot` module captures a window
  (`capture`), crops it (`Shot::crop`, `crop_logical`, `crop_to(&element, margin)`), saves it as
  PNG through a temporary file (`Shot::save_png`), composes captures (`Shot::blank`,
  `Shot::fill`, `Shot::paste`) and sets a window's logical size and scale
  factor for HiDPI pictures (`set_window_size`). Parallel workers use the same renderer.
- `backend::init_backend_with(Renderer)`; `init_backend()` is unchanged (mock renderer).
- `Renderer` is in the prelude.

## slint-harness 0.18.3

Ports the improvements the Indicatrix harness made after the crate was lifted from it.

- Sweep: `SweepOptions::floating_layers` (default `["PopupWindow"]`): controls in a floating layer
  (matched by type name or base) never overlap controls outside it; overlaps inside one layer are
  still reported. A full `SweepOptions { .. }` literal needs the new field (use
  `..SweepOptions::default()`).
- Smoke: `SmokeHost::admits` (default true) restricts a pass to some controls of a view;
  `Smoke::scope_may_close` counts a scoped view (`Window/<scope>`) that closes itself in
  `SmokeStats::closed` instead of a `window-invalid` finding. Every smoke press also credits the
  wrapper component's site id (`coverage::record_wrappers`).
- Coverage: credit by element type and base type, single-site credit for computed labels, label
  literals with escapes and interpolation, ids after an `if`/`for` prefix, one label literal per
  statement, `coverage::record_wrappers` and `coverage::record_site_id`,
  `Coverage::label_props` / `reader_props` / `callback_hint`. Parallel workers carry the new
  credit.

## slint-harness 0.18.2

- Parallel runs (opt-in): `Harness::jobs(Jobs::Auto)` / `Jobs::Fixed(n)` and
  `SLINT_HARNESS_JOBS=auto|<n>`, capped by `Harness::max_jobs` / `SLINT_HARNESS_JOBS_MAX`. The
  test binary starts copies of itself as worker processes, each with its own profile; `Auto`
  starts one worker and adds one every 5 s while at least 1.5 cores were idle. One merged
  report, console output in scenario order. The default stays the serial run.
- New check `worker-crash`: a worker process died during a scenario (parallel runs only).

## slint-harness 0.18.1

- Fix: the default report directory is found from the test binary's path on both Cargo
  layouts (`target/<profile>/deps/` and the newer `target/<profile>/build/<package>/<hash>/out/`)
  before looking for `CACHEDIR.TAG`. 0.18.0 relied on the tag, which a target folder made by an
  older Cargo does not have, and then wrote the report to
  `target/<profile>/build/<package>/slint-harness/<package>/` on nightly Cargo.

## slint-harness 0.18.0

First release. Serves Slint 1.18.1 (exact pin). Lifted from the Indicatrix UI harness.

- Report formats: `Harness::formats([Format::Markdown, Format::Html, Format::Text])` and
  `SLINT_HARNESS_FORMATS=md,html,txt`. `report.json` is always written, `report.md` by
  default; a run removes the format files it did not select.
- The smoke pass recognises modal backdrops with the run's `SweepOptions` (`modal_cover`),
  not the defaults.
- Usability: the first console line names the harness and Slint versions, the selected
  scenarios and the report directory; a failing run prints its findings (up to 25) after the
  per-check counts; a harness with no scenarios is a `config` finding; `WindowView::find*` fall
  back to the accessible placeholder text, so a `LineEdit` is found by its placeholder, and a
  `not-found` finding lists similar placeholders (`WindowView::placeholders`); the abort
  messages for missing debug info, a pre-set platform and a guarded path outside the profile
  say what to do; `slint_harness::SLINT_VERSION` and the `harness` / `slint` fields of
  `report.json`; the prelude adds `ElementRoot`, `Hooks`, `ProfileDir`, `DEFAULT_DENY`,
  `record_callback`, `arm_gate`, `SLINT_VERSION` and `std::process::ExitCode`.
- The HTML report has a `main` landmark and real headings, column headers with scope, a
  per-check summary, and a dark color scheme that follows the system setting.
- Docs: the crate-level example is compile-checked; README quick start and troubleshooting;
  `docs/cookbook.md`; `examples/notes` shows every hook, the coverage gate, deny rules,
  notices and the report formats.
- The default report directory is `<target>/slint-harness/<package>`, one folder per package,
  so two harness tests in one workspace do not delete each other's report files; `<target>`
  follows Cargo's `CACHEDIR.TAG`, so the newer build-dir layout no longer puts reports under
  `target/<profile>/build/`.

## slint-harness-hooks 0.1.0

First release.

- Docs: a doc test of the picker queue that runs; the feature-gated call pattern is compiled
  in `examples/notes`.
