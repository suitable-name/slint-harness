//! `outside-window`: an element with an accessible role lies (partly) outside the window.
//!
//! Only the part a user can SEE counts: the rectangle is first clipped against every enclosing
//! `Flickable` (the viewport of a `ScrollView` / `ListView`), so content scrolled out of its
//! viewport is neither outside the window nor an overlap. An element whose visible part is
//! empty is skipped, as is an element whose combined opacity is at or below
//! [`SweepOptions::invisible_opacity`] (it is not on screen; `invisible-but-enabled` reports it).
//!
//! What remains is real: a viewport that itself sticks out of the window, or content outside
//! any scroll area. For example, in Indicatrix: the Help window's nav rows at y=405 of a
//! 420-high window and a long filter panel in its scroll area were reported before the clipping
//! existed and are not any more; content outside any scroll area is cut off by the window-body
//! `clip: true` rectangle in `ui/app.slint`, which the sweep cannot see (the property is not
//! readable) and does not need to.

use super::{SweepOptions, finding, rect_text};
use crate::{report::Finding, scene::Rect, scene::Scene, view::WindowView};

/// `outside-window` over controls and (optionally) every element that has an accessible role,
/// measured on the part of the element that is inside every enclosing scroll viewport.
pub(super) fn check_bounds(
    view: &WindowView,
    scene: &Scene,
    options: &SweepOptions,
    findings: &mut Vec<Finding>,
) {
    let (window_w, window_h) = view.size;
    let tol = options.bounds_tolerance;
    for placed in &scene.placed {
        let c = &placed.control;
        let v = &placed.visible;
        if v.is_empty() || c.opacity <= options.invisible_opacity {
            continue;
        }
        let outside =
            v.x < -tol || v.y < -tol || v.right() > window_w + tol || v.bottom() > window_h + tol;
        if !outside {
            continue;
        }
        let own = Rect {
            x: c.x,
            y: c.y,
            w: c.width,
            h: c.height,
        };
        let rect = if own.differs(v) {
            format!(
                "{} (visible part of {})",
                rect_text(v.x, v.y, v.w, v.h),
                rect_text(c.x, c.y, c.width, c.height)
            )
        } else {
            rect_text(c.x, c.y, c.width, c.height)
        };
        findings.push(finding(
            view,
            c,
            "outside-window",
            format!("{rect} outside window {window_w:.0}x{window_h:.0}"),
        ));
    }
}
