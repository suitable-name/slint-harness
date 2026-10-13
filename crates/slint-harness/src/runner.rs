//! The harness builder and the run: profile, backend, scenarios (each under `catch_unwind`),
//! coverage gate, report, exit code.
//!
//! Run order: profile (environment redirect, working directory, guarded paths) -> panic
//! recorder -> testing backend -> scenarios -> coverage gate -> profile teardown -> report. The
//! profile comes first because it sets environment variables, which is sound only while the
//! process is single-threaded; the backend and the app create threads.
//!
//! A scenario that panics ends there (a caught panic through Slint callbacks can leave component
//! state unknown), becomes a `scenario-panic` finding, and the run goes on with the next one.
//! A user profile with `panic = "abort"` disables this: the run then ends at the first panic.

use crate::{
    Ctx,
    backend::init_backend,
    checks::SweepOptions,
    coverage::Coverage,
    panics::install_panic_recorder,
    profile::Profile,
    report::{Finding, Format, Report},
    take_panics,
};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    process::ExitCode,
};

/// One named scenario.
pub(crate) type ScenarioFn = Box<dyn FnMut(&mut Ctx<'_>) -> Result<(), Finding>>;

/// The harness: configuration plus the scenario list. Build it as the first statement of the
/// test's `main`, then call [`Harness::run`]. See the crate documentation for a complete `main`.
///
/// Environment: `SLINT_HARNESS_FILTER` (comma-separated name substrings; positional arguments
/// that do not start with `-` filter too), `SLINT_HARNESS_SIZES` (`1366x768,720x480`, replaces
/// [`Harness::sizes`]), `SLINT_HARNESS_FORMATS` (`md,html,txt`; `json` is accepted and ignored
/// because it is always written; replaces [`Harness::formats`]), `SLINT_HARNESS_SEED` (u64, see
/// [`Ctx::seed`]), `SLINT_HARNESS_ALLOW_FAIL` (any value: exit 0 even with findings).
pub struct Harness {
    pub(crate) report_dir: Option<PathBuf>,
    pub(crate) sizes: Vec<(u32, u32)>,
    pub(crate) formats: Vec<Format>,
    pub(crate) profile: Option<Profile>,
    pub(crate) coverage: Option<Coverage>,
    pub(crate) sweep_options: SweepOptions,
    pub(crate) scenarios: Vec<(String, ScenarioFn)>,
    /// Runs after the last scenario and the coverage gate, before the profile is deleted.
    pub(crate) teardown: Option<Box<dyn FnOnce()>>,
    /// The test crate's `CARGO_MANIFEST_DIR` (run-time variable, set by `cargo test`), read
    /// before the profile moves the working directory.
    pub(crate) manifest_dir: PathBuf,
}

impl Harness {
    /// A harness with defaults: report in `<target>/slint-harness/<package>` (`<target>` from
    /// `CARGO_TARGET_TMPDIR` under `cargo test`, else derived from the test binary's path; one
    /// folder per package, so two harness tests in a workspace do not overwrite each other's
    /// files), no extra sizes, an isolated
    /// profile without guarded paths, no coverage gate. Must be the first statement of `main`.
    ///
    /// Creating the harness only records the configuration (the manifest directory is read from
    /// the run-time `CARGO_MANIFEST_DIR`, falling back to the current directory); the profile
    /// is created by [`Harness::run`] before anything starts a thread.
    #[must_use]
    pub fn new() -> Self {
        let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default();
        Self {
            report_dir: None,
            sizes: Vec::new(),
            formats: vec![Format::Markdown],
            profile: None,
            coverage: None,
            sweep_options: SweepOptions::default(),
            scenarios: Vec::new(),
            teardown: None,
            manifest_dir,
        }
    }

    /// Where the report files go (`report.json` plus the selected [`Format`]s; use
    /// `slint_harness::target_dir!()`).
    #[must_use]
    pub fn report_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.report_dir = Some(dir.into());
        self
    }

    /// The window sizes [`Ctx::sweep_at_sizes`] sweeps at; the first is the launch size.
    #[must_use]
    pub fn sizes(mut self, sizes: impl IntoIterator<Item = (u32, u32)>) -> Self {
        self.sizes = sizes.into_iter().collect();
        self
    }

    /// Which report files are written besides `report.json` (always written): the default is
    /// `[Format::Markdown]`, i.e. `report.md`. An empty list writes JSON only. Files of formats
    /// not selected (`report.md`, `report.html`, `report.txt`) left by an earlier run are removed.
    /// `SLINT_HARNESS_FORMATS` replaces this list (see the type doc).
    #[must_use]
    pub fn formats(mut self, formats: impl IntoIterator<Item = Format>) -> Self {
        self.formats = formats.into_iter().collect();
        self
    }

    /// Runs `f` once after the last scenario and the coverage gate, before the profile is
    /// deleted: the place to close windows the scenarios kept alive across scenarios, stop the
    /// app's threads and flush its files (a file still open would keep the profile from being
    /// deleted).
    #[must_use]
    pub fn teardown(mut self, f: impl FnOnce() + 'static) -> Self {
        self.teardown = Some(Box::new(f));
        self
    }

    /// The throw-away profile configuration.
    #[must_use]
    pub fn profile(mut self, profile: Profile) -> Self {
        self.profile = Some(profile);
        self
    }

    /// Turns the coverage gate on.
    #[must_use]
    pub fn coverage(mut self, coverage: Coverage) -> Self {
        self.coverage = Some(coverage);
        self
    }

    /// The sweep thresholds.
    #[must_use]
    pub fn sweep_options(mut self, options: SweepOptions) -> Self {
        self.sweep_options = options;
        self
    }

    /// Adds a scenario. Scenarios run in the order added; an `Err` is recorded as a finding, a
    /// panic as a `scenario-panic` finding, and the run goes on.
    #[must_use]
    pub fn scenario(
        mut self,
        name: impl Into<String>,
        run: impl FnMut(&mut Ctx<'_>) -> Result<(), Finding> + 'static,
    ) -> Self {
        self.scenarios.push((name.into(), Box::new(run)));
        self
    }

    /// Runs everything and returns the exit code: 0 clean (or `SLINT_HARNESS_ALLOW_FAIL`),
    /// 1 findings, 2 aborted (isolation could not be proven, no debug info). A harness with no
    /// scenarios at all is a `config` finding (exit code 1).
    #[must_use]
    pub fn run(self) -> ExitCode {
        let report = self.run_to_report();
        if report.is_empty() {
            return ExitCode::SUCCESS;
        }
        if std::env::var_os("SLINT_HARNESS_ALLOW_FAIL").is_some() {
            println!(
                "slint-harness: {} finding(s) allowed by SLINT_HARNESS_ALLOW_FAIL, exit code 0",
                report.len()
            );
            return ExitCode::SUCCESS;
        }
        ExitCode::FAILURE
    }

    /// Like [`Harness::run`] (profile, backend, scenarios, gate, report files written, profile
    /// deleted) but returns the report instead of deciding the exit code. The self-tests use it
    /// to assert exact finding counts. An abort (exit code 2) still ends the process.
    #[must_use]
    pub fn run_to_report(self) -> Report {
        let Self {
            report_dir,
            sizes,
            formats,
            profile,
            coverage,
            sweep_options,
            mut scenarios,
            mut teardown,
            manifest_dir,
        } = self;
        let (env_filters, arg_filters) = scenario_filters();
        let filters: Vec<String> = env_filters.iter().chain(&arg_filters).cloned().collect();
        let selected = scenarios
            .iter()
            .filter(|(name, _)| {
                filters.is_empty() || filters.iter().any(|f| name.contains(f.as_str()))
            })
            .count();
        let names: Vec<String> = scenarios.iter().map(|(name, _)| name.clone()).collect();
        // The report directory is resolved against the working directory the run starts in
        // (the profile moves it below) and printed first, so the user knows where to look
        // even when the run aborts later.
        let original_cwd = std::env::current_dir().unwrap_or_default();
        let report_dir = resolve_report_dir(report_dir, &original_cwd);
        print_banner(selected, scenarios.len(), &report_dir);
        if selected == 0 && env_filters.is_empty() && !arg_filters.is_empty() {
            println!(
                "slint-harness: no scenario matches the argument filter {arg_filters:?} \
                 (scenarios: {names:?}); nothing run, report not written"
            );
            return Report::new(report_dir);
        }
        // 1. Isolation first, while the process is still single-threaded.
        let profile = profile.unwrap_or_else(Profile::isolated).create();
        // 2. Testing backend, then the panic recorder (so a platform panic is not recorded as
        // an app panic).
        if let Err(message) = init_backend() {
            profile.finish();
            crate::profile::abort_run(&message);
        }
        install_panic_recorder();

        let mut report = Report::new(report_dir);
        let sizes = resolve_sizes(sizes, &mut report);
        let formats = resolve_formats(formats, &mut report);
        report.set_formats(formats);
        config_findings(&mut report, &env_filters, selected, &names);
        let seed = std::env::var("SLINT_HARNESS_SEED")
            .ok()
            .and_then(|text| text.trim().parse::<u64>().ok())
            .unwrap_or(0);

        // 3. Scenarios.
        for (name, run) in &mut scenarios {
            if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
                continue;
            }
            report.begin_scenario(name);
            let _ = take_panics();
            crate::hooks::picker::clear();
            let outcome = {
                let mut ctx = Ctx {
                    report: &mut report,
                    profile: &profile,
                    sizes: &sizes,
                    sweep_options: &sweep_options,
                    seed,
                    notice: None,
                    clear_notice: None,
                };
                catch_unwind(AssertUnwindSafe(|| {
                    if let Err(finding) = run(&mut ctx) {
                        ctx.report.add(finding);
                    }
                    ctx.check_health("end of scenario");
                }))
            };
            if outcome.is_err() {
                let recorded = take_panics().join(" | ");
                report.add(Finding::simple(
                    "scenario-panic",
                    format!("{name} aborted: {recorded}"),
                ));
            }
            report.end_scenario(name);
            if let Some(message) = crate::profile::take_abort_request() {
                if let Some(teardown) = teardown.take() {
                    teardown();
                }
                profile.finish();
                crate::profile::abort_run(&message);
            }
        }

        // 4. Coverage gate, teardown and report.
        if let Some(coverage) = &coverage {
            report.set_context("coverage");
            crate::coverage::finish(coverage, &manifest_dir, !filters.is_empty(), &mut report);
        }
        if let Some(teardown) = teardown {
            teardown();
        }
        profile.finish();
        report.finish();
        report
    }
}

impl Default for Harness {
    fn default() -> Self {
        Self::new()
    }
}

/// The first line of every run: harness and Slint versions, how many scenarios the filters
/// selected, and where the report files go (printed before anything can abort).
fn print_banner(selected: usize, total: usize, report_dir: &Path) {
    println!(
        "slint-harness {} (Slint {}): {selected} of {total} scenario(s) selected, report dir {}",
        env!("CARGO_PKG_VERSION"),
        crate::SLINT_VERSION,
        report_dir.display()
    );
}

/// The `config` findings about the scenario list: a harness with no scenarios at all (a
/// forgotten `Harness::scenario` would otherwise pass silently) and a `SLINT_HARNESS_FILTER`
/// that matches none of them.
fn config_findings(report: &mut Report, env_filters: &[String], selected: usize, names: &[String]) {
    if names.is_empty() {
        report.add(Finding::simple(
            "config",
            "no scenarios: add at least one Harness::scenario(..) before run()",
        ));
    }
    if selected == 0 && !env_filters.is_empty() {
        report.add(Finding::simple(
            "config",
            format!(
                "SLINT_HARNESS_FILTER={:?} matches no scenario (scenarios: {names:?})",
                env_filters.join(",")
            ),
        ));
    }
}

/// The configured report directory (a relative one resolved against the ORIGINAL working
/// directory, the profile has moved it), or `<target>/slint-harness/<package>`: the target dir
/// from `CARGO_TARGET_TMPDIR` first, then derived from the test binary (see `target_dir_of`),
/// and one folder per package. Without the folder, two harness tests in one workspace (the
/// `counter` and `notes` examples here) wrote the same directory, and each run's removal of the
/// formats it did not select deleted the other's files.
fn resolve_report_dir(configured: Option<PathBuf>, original_cwd: &Path) -> PathBuf {
    if let Some(dir) = configured {
        return if dir.is_absolute() {
            dir
        } else {
            original_cwd.join(dir)
        };
    }
    let base = target_dir_of(
        std::env::var_os("CARGO_TARGET_TMPDIR")
            .map(PathBuf::from)
            .as_deref(),
        std::env::current_exe().ok().as_deref(),
        original_cwd,
    )
    .join("slint-harness");
    match package_folder() {
        Some(package) => base.join(package),
        None => base,
    }
}

/// The folder name of the package under test: the run-time `CARGO_PKG_NAME` (`cargo test` sets
/// it for the test process), else the name of the `CARGO_MANIFEST_DIR` folder, else none (a
/// binary run outside cargo writes straight into `<target>/slint-harness`).
fn package_folder() -> Option<String> {
    std::env::var("CARGO_PKG_NAME")
        .ok()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            std::env::var_os("CARGO_MANIFEST_DIR").and_then(|dir| {
                Path::new(&dir)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
        })
}

/// The cargo target directory: the parent of `CARGO_TARGET_TMPDIR` (`compile_time` is the
/// value `target_dir!` captured, else the run-time variable), else derived from the test
/// binary, else `target` under the working directory. From the binary, the first ancestor that
/// holds Cargo's `CACHEDIR.TAG` wins (Cargo writes it at the target-dir root only), which also
/// covers the newer build-dir layout where test binaries live in
/// `target/<profile>/build/<package>/<hash>/out/`; without the tag the old layout is assumed
/// (`target/<profile>/deps/<test>` -> `target`).
#[doc(hidden)]
#[must_use]
pub fn target_dir_from(compile_time: Option<&str>) -> PathBuf {
    let tmpdir = compile_time
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("CARGO_TARGET_TMPDIR").map(PathBuf::from));
    let cwd = std::env::current_dir().unwrap_or_default();
    target_dir_of(
        tmpdir.as_deref(),
        std::env::current_exe().ok().as_deref(),
        &cwd,
    )
}

fn target_dir_of(tmpdir: Option<&Path>, exe: Option<&Path>, cwd: &Path) -> PathBuf {
    if let Some(parent) = tmpdir.and_then(Path::parent) {
        return parent.to_path_buf();
    }
    let tagged = exe.and_then(|exe| {
        exe.ancestors()
            .skip(1)
            .find(|dir| dir.join("CACHEDIR.TAG").is_file())
    });
    tagged
        .or_else(|| exe.and_then(|exe| exe.parent().and_then(Path::parent).and_then(Path::parent)))
        .map_or_else(|| cwd.join("target"), Path::to_path_buf)
}

/// The window sizes of the run: `SLINT_HARNESS_SIZES` (`WxH,WxH`) when set and valid, else
/// `configured`. Invalid text is a `config` finding and the configured list is kept.
fn resolve_sizes(configured: Vec<(u32, u32)>, report: &mut Report) -> Vec<(u32, u32)> {
    let Ok(text) = std::env::var("SLINT_HARNESS_SIZES") else {
        return configured;
    };
    if let Some(sizes) = parse_sizes(&text) {
        return sizes;
    }
    report.add(Finding::simple(
        "config",
        format!("SLINT_HARNESS_SIZES={text:?} is not a list like \"1366x768,720x480\"; ignored"),
    ));
    configured
}

/// Parses `WxH,WxH`; `None` when any entry is malformed or a side is zero.
fn parse_sizes(text: &str) -> Option<Vec<(u32, u32)>> {
    let sizes: Option<Vec<(u32, u32)>> = text
        .split(',')
        .map(|entry| {
            let (width, height) = entry.trim().split_once(['x', 'X'])?;
            let width: u32 = width.trim().parse().ok()?;
            let height: u32 = height.trim().parse().ok()?;
            (width > 0 && height > 0).then_some((width, height))
        })
        .collect();
    sizes.filter(|sizes| !sizes.is_empty())
}

/// The report formats of the run: `SLINT_HARNESS_FORMATS` when set and valid, else
/// `configured`. Invalid text is a `config` finding and the configured list is kept.
fn resolve_formats(configured: Vec<Format>, report: &mut Report) -> Vec<Format> {
    let Ok(text) = std::env::var("SLINT_HARNESS_FORMATS") else {
        return configured;
    };
    if let Some(formats) = parse_formats(&text) {
        return formats;
    }
    report.add(Finding::simple(
        "config",
        format!(
            "SLINT_HARNESS_FORMATS={text:?} is not a list like \"md,html,txt\" (json is \
             always written); ignored"
        ),
    ));
    configured
}

/// Parses `md,html,txt` (names as [`Format::from_name`]; `json` is accepted and skipped,
/// duplicates dropped, first-appearance order kept). `None` on an unknown name; an empty
/// list is `Some(vec![])` (JSON only).
fn parse_formats(text: &str) -> Option<Vec<Format>> {
    let mut formats = Vec::new();
    for token in text.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        if token.eq_ignore_ascii_case("json") {
            continue;
        }
        let format = Format::from_name(token)?;
        if !formats.contains(&format) {
            formats.push(format);
        }
    }
    Some(formats)
}

/// Scenario name filters: `SLINT_HARNESS_FILTER` (comma-separated) and the positional
/// arguments that do not start with `-`, kept apart because they mean different things when
/// nothing matches (see `run_to_report`).
fn scenario_filters() -> (Vec<String>, Vec<String>) {
    split_filters(
        std::env::var("SLINT_HARNESS_FILTER").ok(),
        std::env::args_os()
            .skip(1)
            .map(|a| a.to_string_lossy().into_owned()),
    )
}

fn split_filters(
    env: Option<String>,
    args: impl Iterator<Item = String>,
) -> (Vec<String>, Vec<String>) {
    let from_env: Vec<String> = env
        .map(|text| {
            text.split(',')
                .map(|part| part.trim().to_owned())
                .filter(|part| !part.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let from_args: Vec<String> = args.filter(|a| !a.starts_with('-')).collect();
    (from_env, from_args)
}

#[cfg(test)]
mod tests {
    use super::{package_folder, parse_formats, parse_sizes, split_filters, target_dir_of};
    use crate::report::Format;
    use std::path::{Path, PathBuf};

    #[test]
    fn parse_formats_accepts_names_skips_json_and_dedupes() {
        assert_eq!(
            parse_formats("md, HTML ,json,txt,md"),
            Some(vec![Format::Markdown, Format::Html, Format::Text])
        );
        assert_eq!(parse_formats("raw"), Some(vec![Format::Text]));
        assert_eq!(parse_formats(""), Some(vec![]));
        assert_eq!(parse_formats("json"), Some(vec![]));
        assert_eq!(parse_formats("pdf"), None);
        assert_eq!(parse_formats("md,pdf"), None);
    }

    #[test]
    fn split_filters_separates_env_and_arguments() {
        let (env, args) = split_filters(
            Some(" a, ,b ".to_owned()),
            ["--nocapture", "c"].into_iter().map(str::to_owned),
        );
        assert_eq!(env, ["a", "b"]);
        assert_eq!(args, ["c"]);
        let (env, args) = split_filters(None, std::iter::empty());
        assert!(env.is_empty() && args.is_empty());
    }

    #[test]
    fn parse_sizes_accepts_lists_and_rejects_garbage() {
        assert_eq!(
            parse_sizes("1366x768, 720X480"),
            Some(vec![(1366, 768), (720, 480)])
        );
        assert_eq!(parse_sizes("0x10"), None);
        assert_eq!(parse_sizes("abc"), None);
        assert_eq!(parse_sizes(""), None);
    }

    #[test]
    fn target_dir_resolution_order() {
        let from_tmpdir = target_dir_of(Some(Path::new("/t/target/tmp")), None, Path::new("/cwd"));
        assert_eq!(from_tmpdir, PathBuf::from("/t/target"));
        let from_exe = target_dir_of(
            None,
            Some(Path::new("/t/target/debug/deps/x")),
            Path::new("/cwd"),
        );
        assert_eq!(from_exe, PathBuf::from("/t/target"));
        let from_cwd = target_dir_of(None, None, Path::new("/cwd"));
        assert_eq!(from_cwd, PathBuf::from("/cwd").join("target"));
    }

    /// `cargo test` sets `CARGO_PKG_NAME` for the test process, so the default report folder is
    /// the package's own (two harness tests in one workspace no longer share one directory).
    #[test]
    fn package_folder_is_the_package_under_test() {
        assert_eq!(package_folder().as_deref(), Some(env!("CARGO_PKG_NAME")));
    }

    /// The newer build-dir layout puts test binaries four levels below the target dir; the
    /// `CACHEDIR.TAG` Cargo writes at the target-dir root finds it, where the three-parents rule
    /// would land inside `build/` (the stray `target/debug/build/<pkg>/slint-harness` reports).
    #[test]
    fn target_dir_follows_the_cachedir_tag() {
        let root = std::env::temp_dir().join(format!(
            "slint-harness-unit-cachedir-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let target = root.join("target");
        let out = target
            .join("debug")
            .join("build")
            .join("app")
            .join("0123")
            .join("out");
        let made = std::fs::create_dir_all(&out)
            .and_then(|()| std::fs::write(target.join("CACHEDIR.TAG"), "Signature: x"));
        let found = target_dir_of(None, Some(&out.join("ui.exe")), Path::new("/cwd"));
        let _ = std::fs::remove_dir_all(&root);
        assert!(made.is_ok(), "could not build the fixture tree: {made:?}");
        assert_eq!(found, target);
    }
}
