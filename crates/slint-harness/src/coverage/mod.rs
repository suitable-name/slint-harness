//! The coverage gate: every UI action is exercised by the run, or the run fails.
//!
//! Two inventories are read statically from the `.slint` files under [`Coverage::scan`]'s
//! directory:
//!
//! * **callbacks**: every `callback` declared in a `global` that has no Slint body, as
//!   `Model.name`;
//! * **interactive elements**: every INSTANTIATION SITE of an interactive type, plus every inline
//!   element that declares an interactive `accessible-role` itself (button, checkbox, switch,
//!   combobox, slider, spinbox, tab, text-input). Interactive types are the std widgets
//!   (`Button`, `CheckBox`, `ComboBox`, `LineEdit` ...), the app components whose root declares
//!   such a role, WRAPPER components (one interactive element inside whose label reads
//!   `root.<property>`, for example, in Indicatrix, `TipButton`) and everything that inherits one
//!   of those. A component's own root or wrapped inner element is not an entry: its uses are.
//!   One entry per site, so a site in a `for` template counts once.
//!
//! During the run, [`record_activation`] (smoke clicks and `WindowView::click`),
//! [`record_touch`] (a scenario fetched the element to act on it) and [`record_callback`] (a
//! scenario invoked a global callback and asserted its effect) fill a recorder. `finish` runs
//! at the end of the run, compares recorder and inventory, and turns every item that is neither
//! covered nor on the allow list into a `coverage` finding. It also adds the "Coverage" section
//! to the report.
//!
//! Element matching is fuzzy by nature: an inventoried element is matched by its
//! `Component::id` first, then by element type plus the label the site gives it (`text:` /
//! `label:` ... or a literal `accessible-label`). An element with neither (no `id :=`, label
//! computed by an expression) cannot be tracked; the section counts and lists them (each is a
//! candidate for an `id` in the UI) but they are not findings.
//!
//! The matching rules (single-site credit, ids, label pairs, escapes and interpolations in
//! literals) are in `matching.rs`; the extension points for apps with their own label properties
//! are [`Coverage::label_props`] and [`Coverage::reader_props`]; scenarios that press a wrapper
//! component by hand credit it with [`record_wrappers`] or [`record_site_id`].
//!
//! Known limits of the text scanner behind the inventory: component names must start with a
//! capital letter, `import { X as Y }` renames are not followed, `@children` slots and `states`
//! blocks are opaque, and labels built by expressions are untrackable. See the inventory module.
//!
//! The gate is opt-in (`Harness::coverage`). It runs per [`GateRule`]: by default only when no
//! scenario filter is active, because a filtered run would report everything as uncovered.

mod allow;
mod inventory;
mod matching;
mod parse;
mod props;
pub mod recorder;
mod scan;

pub use recorder::{
    record_activation, record_callback, record_site_id, record_touch, record_wrappers,
    recorded_callbacks,
};

use crate::report::{Finding, Report};
use allow::AllowList;
use inventory::load_inventory;
use matching::{Reach, reach_of, single_site_types};
use props::LabelProps;
use recorder::{Recorder, recorder};
use std::{
    collections::BTreeSet,
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

/// When the end-of-run gate runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GateRule {
    /// Only when no scenario filter is active (a filtered run would report everything as
    /// uncovered). The default.
    #[default]
    NoFilter,
    /// Always.
    Always,
    /// Only when a scenario called [`arm_gate`] (Indicatrix: the direct-invoke scenario).
    Armed,
}

/// Configuration of the coverage gate (a builder).
#[derive(Debug, Clone)]
pub struct Coverage {
    /// The `.slint` source directory, relative to the test crate's `CARGO_MANIFEST_DIR`.
    pub(crate) scan_root: PathBuf,
    /// A TOML allow file (`[[allow]] key = "...", reason = "..."`), relative to the manifest dir.
    pub(crate) allow_file: Option<PathBuf>,
    /// In-code allow entries `(key, reason)`; a key ending in `*` is a prefix.
    pub(crate) allow: Vec<(String, String)>,
    /// When the gate runs.
    pub(crate) gate: GateRule,
    /// Extra property names read as labels, and the subset that beats `text` on wrappers.
    pub(crate) props: LabelProps,
    /// The advice at the end of a callback-gap finding.
    pub(crate) callback_hint: String,
}

/// The default advice of a callback-gap finding.
const DEFAULT_CALLBACK_HINT: &str =
    "add a scenario that invokes it or an allow entry with a reason";

impl Coverage {
    /// Scans `dir/**/*.slint` (relative to the test crate's manifest directory).
    #[must_use]
    pub fn scan(dir: impl Into<PathBuf>) -> Self {
        Self {
            scan_root: dir.into(),
            allow_file: None,
            allow: Vec::new(),
            gate: GateRule::default(),
            props: LabelProps::default(),
            callback_hint: DEFAULT_CALLBACK_HINT.to_owned(),
        }
    }

    /// Reads allow entries from this TOML file too.
    #[must_use]
    pub fn allow_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.allow_file = Some(path.into());
        self
    }

    /// Adds in-code allow entries `(key, reason)`.
    #[must_use]
    pub fn allow(mut self, entries: &[(&str, &str)]) -> Self {
        self.allow.extend(
            entries
                .iter()
                .map(|(key, reason)| ((*key).to_owned(), (*reason).to_owned())),
        );
        self
    }

    /// Reads these property names as labels too, in addition to the std list (`accessible-label`,
    /// `text`, `label`, `title`, `hint`, `name`). For an app whose wrapper components carry the
    /// label the screen reader announces in a property of their own (for example, in Indicatrix,
    /// `reader_label` and `slider_label`). Names match exactly as spelled in the source.
    #[must_use]
    pub fn label_props(mut self, names: &[&str]) -> Self {
        self.props
            .extra
            .extend(names.iter().map(|n| (*n).to_owned()));
        self
    }

    /// Properties whose LITERAL wins over `text` as the label of a non-std wrapper site, because
    /// the running accessible label is that property when the site sets one (for example, in
    /// Indicatrix, `reader_label` on `TipButton`). They are read as labels too, so naming a
    /// property here is enough; `label_props` alone never overrides `text`.
    #[must_use]
    pub fn reader_props(mut self, names: &[&str]) -> Self {
        self.props
            .reader
            .extend(names.iter().map(|n| (*n).to_owned()));
        self
    }

    /// Replaces the advice at the end of a callback-gap finding, whose detail reads
    /// `callback never invoked ({file}:{line}); {hint}`. The default is "add a scenario that
    /// invokes it or an allow entry with a reason".
    #[must_use]
    pub fn callback_hint(mut self, hint: impl Into<String>) -> Self {
        self.callback_hint = hint.into();
        self
    }

    /// Sets when the gate runs.
    #[must_use]
    pub const fn gate_when(mut self, rule: GateRule) -> Self {
        self.gate = rule;
        self
    }
}

/// Set by [`arm_gate`]; read by `finish` under [`GateRule::Armed`].
static ARMED: AtomicBool = AtomicBool::new(false);

/// Arms the end-of-run gate for [`GateRule::Armed`] (for example, in Indicatrix, the
/// direct-invoke scenario calls it: when that scenario did not run, the run was filtered).
pub fn arm_gate() {
    ARMED.store(true, Ordering::SeqCst);
}

/// Whether [`arm_gate`] was called in this process (a parallel run reads it from each worker).
pub(crate) fn is_armed() -> bool {
    ARMED.load(Ordering::SeqCst)
}

/// Sets the armed flag (a parallel run: a worker's scenario called [`arm_gate`]).
pub(crate) fn set_armed(armed: bool) {
    ARMED.store(armed, Ordering::SeqCst);
}

fn push_list(out: &mut String, title: &str, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n#### {title} ({})\n", lines.len());
    for line in lines {
        let _ = writeln!(out, "- {line}");
    }
}

/// Counters and lists of one gate evaluation.
#[derive(Default)]
struct Outcome {
    callbacks: usize,
    callbacks_invoked: usize,
    callbacks_allowed: usize,
    elements: usize,
    activated: usize,
    touched: usize,
    allowed: usize,
    untrackable: usize,
    uncovered_callbacks: Vec<String>,
    uncovered_elements: Vec<String>,
    untrackable_lines: Vec<String>,
    allowed_lines: Vec<String>,
    stale: Vec<String>,
    findings: Vec<Finding>,
}

/// Compares the recorder with the inventory and the allow list.
fn evaluate(
    inventory: &inventory::Inventory,
    allow: &AllowList,
    rec: &Recorder,
    callback_hint: &str,
) -> Outcome {
    let mut out = Outcome {
        callbacks: inventory.callbacks.len(),
        elements: inventory.elements.len(),
        ..Outcome::default()
    };
    let mut keys: BTreeSet<String> = BTreeSet::new();
    let single = single_site_types(inventory);

    for item in &inventory.callbacks {
        keys.insert(item.key.clone());
        if rec.callbacks.contains(&item.key) {
            out.callbacks_invoked += 1;
        } else if let Some(reason) = allow.reason(&item.key) {
            out.callbacks_allowed += 1;
            out.allowed_lines.push(format!(
                "callback {} ({}:{}): {reason}",
                item.key, item.file, item.line
            ));
        } else {
            out.uncovered_callbacks
                .push(format!("{} ({}:{})", item.key, item.file, item.line));
            out.findings.push(Finding::new(
                "-",
                item.key.clone(),
                "coverage",
                format!(
                    "callback never invoked ({}:{}); {callback_hint}",
                    item.file, item.line
                ),
            ));
        }
    }

    for item in &inventory.elements {
        let key = item.key();
        keys.insert(key.clone());
        match reach_of(item, rec, &single) {
            Reach::Activated => out.activated += 1,
            Reach::Touched => out.touched += 1,
            Reach::Missed => {
                if let Some(reason) = allow.reason(&key) {
                    out.allowed += 1;
                    out.allowed_lines
                        .push(format!("element {} : {reason}", item.describe()));
                } else if !item.trackable() {
                    out.untrackable += 1;
                    out.untrackable_lines.push(item.describe());
                } else {
                    out.uncovered_elements.push(item.describe());
                    out.findings.push(Finding::new(
                        "-",
                        key,
                        "coverage",
                        format!(
                            "interactive element never activated ({}:{}, {} {})",
                            item.file, item.line, item.role, item.ty
                        ),
                    ));
                }
            }
        }
    }

    // Stale allow-list entries: nothing in the inventory matches them any more.
    for (entry, reason) in allow.stale(&keys) {
        out.stale.push(format!("{entry} : {reason}"));
        out.findings.push(Finding::new(
            "-",
            entry,
            "coverage",
            format!("stale allow-list entry (matches nothing in the inventory): {reason}"),
        ));
    }
    out
}

/// The Markdown body of the report's "Coverage" section.
fn render_section(out: &Outcome) -> String {
    let mut body = String::new();
    let _ = writeln!(
        body,
        "- Callbacks: {} invoked / {} inventoried ({} allow-listed, {} uncovered).",
        out.callbacks_invoked,
        out.callbacks,
        out.callbacks_allowed,
        out.uncovered_callbacks.len()
    );
    let _ = writeln!(
        body,
        "- Interactive elements: {} activated + {} touched (fetched by a scenario) / {} \
         inventoried ({} allow-listed, {} uncovered, {} untrackable: no id and no literal label).",
        out.activated,
        out.touched,
        out.elements,
        out.allowed,
        out.uncovered_elements.len(),
        out.untrackable
    );
    let _ = writeln!(body, "- Stale allow-list entries: {}.", out.stale.len());
    push_list(&mut body, "Uncovered callbacks", &out.uncovered_callbacks);
    push_list(&mut body, "Uncovered elements", &out.uncovered_elements);
    push_list(&mut body, "Stale allow-list entries", &out.stale);
    push_list(&mut body, "Allow-listed (reason)", &out.allowed_lines);
    push_list(
        &mut body,
        "Untrackable elements (add an `id :=` or a literal accessible-label to track them)",
        &out.untrackable_lines,
    );
    body
}

/// Runs the gate at the end of the run (when `coverage.gate` says so): fills the report's
/// Coverage section and adds a `coverage` finding for every uncovered item that is not
/// allow-listed, and for every stale allow entry. `manifest_dir` is the test crate's
/// `CARGO_MANIFEST_DIR`, captured before the profile moved the working directory; `filtered`
/// says whether a scenario filter was active.
///
/// A scan root with no callbacks and no interactive elements (usually a wrong directory) and an
/// unreadable or unparsable allow file are `coverage` findings too.
pub(crate) fn finish(
    coverage: &Coverage,
    manifest_dir: &Path,
    filtered: bool,
    report: &mut Report,
) {
    match coverage.gate {
        GateRule::NoFilter if filtered => {
            report.note("coverage gate skipped: a scenario filter is active");
            return;
        }
        GateRule::Armed if !ARMED.load(Ordering::SeqCst) => {
            report.note(
                "coverage gate skipped: not armed (GateRule::Armed and no scenario called \
                 arm_gate)",
            );
            return;
        }
        _ => {}
    }
    let root = manifest_dir.join(&coverage.scan_root);
    let inventory = load_inventory(&root, &coverage.props);
    if inventory.callbacks.is_empty() && inventory.elements.is_empty() {
        report.add(Finding::new(
            "-",
            coverage.scan_root.display().to_string(),
            "coverage",
            format!(
                "the scan root {} holds no global callbacks and no interactive elements \
                 (wrong directory?)",
                root.display()
            ),
        ));
        return;
    }
    let allow_path = coverage.allow_file.as_ref().map(|f| manifest_dir.join(f));
    let (allow, allow_error) = AllowList::load(&coverage.allow, allow_path.as_deref());
    let outcome = {
        let rec = recorder();
        evaluate(&inventory, &allow, &rec, &coverage.callback_hint)
    };
    report.add_section("Coverage", render_section(&outcome));
    println!(
        "slint-harness: coverage: callbacks {}/{} (+{} allowed), elements {}+{}/{} (+{} allowed, {} untrackable), {} gap finding(s)",
        outcome.callbacks_invoked,
        outcome.callbacks,
        outcome.callbacks_allowed,
        outcome.activated,
        outcome.touched,
        outcome.elements,
        outcome.allowed,
        outcome.untrackable,
        outcome.findings.len()
    );
    if let (Some(error), Some(path)) = (allow_error, allow_path) {
        report.add(Finding::new(
            "-",
            path.display().to_string(),
            "coverage",
            error,
        ));
    }
    for finding in outcome.findings {
        report.add(finding);
    }
}
