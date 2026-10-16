//! `zero-size`, `missing-label` and `invisible-but-enabled`: the per-control checks, one pass
//! over each visible interactive control.
//!
//! * `zero-size`: the control has no width or no height, so a user cannot see or click it;
//! * `missing-label`: the control has no accessible label, so a screen reader announces
//!   nothing. A placeholder text counts as a label (the accessible label of a `LineEdit` falls
//!   back to it);
//! * `invisible-but-enabled`: the control is enabled and its combined opacity (its own and its
//!   ancestors') is at or below [`SweepOptions::invisible_opacity`]: a keyboard user can still
//!   reach what a mouse user cannot see. Such a control is left out of the bounds and overlap
//!   checks, because it is not on screen.

use super::{SweepOptions, finding, rect_text};
use crate::{
    report::Finding,
    view::{Control, WindowView},
};

/// `zero-size`, `missing-label` and `invisible-but-enabled` for one control.
pub(super) fn check_control(
    view: &WindowView,
    control: &Control,
    options: &SweepOptions,
    findings: &mut Vec<Finding>,
) {
    if control.width <= 0.0 || control.height <= 0.0 {
        findings.push(finding(
            view,
            control,
            "zero-size",
            format!("size {:.1}x{:.1}", control.width, control.height),
        ));
    }
    if control.label.trim().is_empty() {
        findings.push(finding(
            view,
            control,
            "missing-label",
            format!(
                "no accessible label ({})",
                rect_text(control.x, control.y, control.width, control.height)
            ),
        ));
    }
    if control.enabled && control.opacity <= options.invisible_opacity {
        findings.push(finding(
            view,
            control,
            "invisible-but-enabled",
            format!("opacity {:.2} but enabled", control.opacity),
        ));
    }
}
