# Changelog

## slint-harness 0.18.0 (unreleased)

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

## slint-harness-hooks 0.1.0 (unreleased)

First release.

- Docs: a doc test of the picker queue that runs; the feature-gated call pattern is compiled
  in `examples/notes`.
