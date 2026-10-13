//! The invariant sweep, run on every window state a scenario reaches.
//!
//! For one [`WindowView`] it checks, over the visible elements Slint reports:
//!
//! * `outside-window` (`bounds.rs`): an element with an accessible role lies (partly) outside
//!   the window, measured on the part a user can see (clipped against every enclosing scroll
//!   viewport);
//! * `zero-size` (`control.rs`): a visible interactive control has no width or no height;
//! * `overlap` (`overlap.rs`): two enabled interactive controls overlap partially, measured on
//!   their visible parts; containment, controls under a modal backdrop and controls laid on a
//!   header-size `TouchArea` on purpose are not overlaps;
//! * `missing-label` (`control.rs`): a visible interactive control has no accessible label (a
//!   placeholder text counts as a label);
//! * `invisible-but-enabled` (`control.rs`): an enabled interactive control whose combined
//!   opacity is zero, which a keyboard user can still reach; it is left out of the bounds and
//!   overlap checks.
//!
//! The full text of each rule, with the real cases that shaped it, is in the documentation of
//! its file. Thresholds are [`SweepOptions`]; the defaults reproduce the Indicatrix sweep.
//!
//! The geometry is measured by Slint's testing backend with the system fonts, so the exact
//! numbers (and the findings that depend on them) differ between machines; see the crate
//! documentation. The scale factor is always 1.

mod bounds;
mod control;
mod overlap;

use crate::{
    report::Finding,
    scene::Scene,
    view::{Control, WindowView},
};

/// Thresholds of the sweep.
#[derive(Debug, Clone)]
pub struct SweepOptions {
    /// Pixels an element may stick out of the window before it is reported.
    pub bounds_tolerance: f32,
    /// Minimum overlap in both axes (pixels) before two controls count as overlapping.
    pub overlap_tolerance: f32,
    /// Opacity at or below which an element counts as invisible.
    pub invisible_opacity: f32,
    /// Whether to run the (quadratic) overlap check.
    pub check_overlap: bool,
    /// Also check the bounds of every element with an accessible role, not only controls.
    pub bounds_for_all_roles: bool,
    /// Fraction of the window a role-less `TouchArea` must cover to count as a modal backdrop.
    pub modal_cover: f32,
    /// Maximum height (pixels) of a header-size `TouchArea` backdrop.
    pub header_max_h: f32,
    /// Fraction of a control that must lie inside a header backdrop to be "laid on it".
    pub header_inside: f32,
}

impl Default for SweepOptions {
    fn default() -> Self {
        Self {
            bounds_tolerance: 1.0,
            overlap_tolerance: 1.5,
            invisible_opacity: 0.01,
            check_overlap: true,
            bounds_for_all_roles: true,
            modal_cover: 0.25,
            header_max_h: 32.0,
            header_inside: 0.75,
        }
    }
}

/// Runs every check on `view` and returns the findings (empty when clean).
#[must_use]
pub fn sweep(view: &WindowView, options: &SweepOptions) -> Vec<Finding> {
    let mut findings = Vec::new();
    let scene = Scene::build(view, options);
    bounds::check_bounds(view, &scene, options, &mut findings);
    for placed in scene.placed.iter().filter(|p| p.interactive) {
        control::check_control(view, &placed.control, options, &mut findings);
    }
    if options.check_overlap {
        overlap::check_overlaps(view, &scene, options, &mut findings);
    }
    findings
}

pub(super) fn finding(
    view: &WindowView,
    control: &Control,
    check: &str,
    detail: String,
) -> Finding {
    Finding::new(view.name.clone(), control.describe(), check, detail)
}

pub(super) fn rect_text(x: f32, y: f32, w: f32, h: f32) -> String {
    format!("rect x={x:.0} y={y:.0} w={w:.0} h={h:.0}")
}
