//! How the recorder reaches an inventoried element: the gate's matching rules.
//!
//! An element is credited by, in this order:
//!
//! 1. **single-site credit**: its label is not a literal, its `match_ty` has exactly one site in
//!    the whole inventory, and the run activated (or fetched) an element of that type; an app
//!    component (declared under the scan root) is also credited by the BASE names of an activated
//!    element. For example, in Indicatrix, a `for` row of a computed-label wrapper with one site:
//!    nothing else can be the element of that type. A type with several sites is never credited
//!    this way, and a builtin base (`Rectangle`, `TouchArea`) never is either;
//! 2. **id**: its `Component::id` was recorded;
//! 3. **label pair**: a recorded `(type, label)` pair whose type is the site's `match_ty` or its
//!    own type (`ty`: a wrapper site keyed by the wrapper's name) and whose label matches the
//!    site's literal ([`literal_matches`]);
//! 4. **label alone** (inline elements that declare their role themselves only).
//!
//! Literals are compared as the running element shows them: escapes (`\u{2026}`, `\"`, `\\`,
//! `\n`) are resolved, a `\{expr}` interpolation splits the literal into fixed pieces, and an
//! interpolated literal needs at least three characters of fixed text to match anything.

use super::{
    inventory::{ElementItem, Inventory, Label},
    recorder::Recorder,
};
use std::collections::{BTreeMap, BTreeSet};

/// How the run reached an element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Reach {
    Activated,
    Touched,
    Missed,
}

/// The single-site types: `all` are credited by the element's own type name; `by_base` (app
/// components only, never a builtin like `Rectangle` or `TouchArea`) also by the base names of an
/// activated element, which is how an instance of a component whose root is the interactive
/// element is recognised.
pub(super) struct Singles {
    all: BTreeSet<String>,
    by_base: BTreeSet<String>,
}

/// Element types that have exactly ONE instantiation site in the whole inventory. A site of such a
/// type whose label is computed (`for` rows, state-dependent text) cannot be told apart by label,
/// but nothing else can be the element of that type: an activated element of the type is that
/// site. (A type with several sites is never recognised this way.)
pub(super) fn single_site_types(inventory: &Inventory) -> Singles {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for item in &inventory.elements {
        *counts.entry(item.match_ty.as_str()).or_insert(0) += 1;
    }
    let all: BTreeSet<String> = counts
        .into_iter()
        .filter(|(_, count)| *count == 1)
        .map(|(ty, _)| ty.to_owned())
        .collect();
    // An inheritor of an interactive component shares its `match_ty` and has its own sites, so a
    // component with inheritors in use is never single; the base-name credit is therefore exact
    // for the app components that stay in this set.
    let components = inventory.component_names();
    let by_base = all
        .iter()
        .filter(|ty| components.contains(*ty))
        .cloned()
        .collect();
    Singles { all, by_base }
}

/// How the run reached `item` (see the module docs for the order of the rules).
pub(super) fn reach_of(item: &ElementItem, rec: &Recorder, single: &Singles) -> Reach {
    if !matches!(item.label, Label::Literal(_)) && single.all.contains(&item.match_ty) {
        let by_base = single.by_base.contains(&item.match_ty);
        if rec.activated_types.contains(&item.match_ty)
            || (by_base && rec.activated_bases.contains(&item.match_ty))
        {
            return Reach::Activated;
        }
        if rec.touched_types.contains(&item.match_ty)
            || (by_base && rec.touched_bases.contains(&item.match_ty))
        {
            return Reach::Touched;
        }
    }
    if let Some(id) = item.runtime_id() {
        if rec.activated_ids.contains(&id) {
            return Reach::Activated;
        }
        if rec.touched_ids.contains(&id) {
            return Reach::Touched;
        }
    }
    if let Label::Literal(text) = &item.label {
        let hit_pair = |pairs: &BTreeSet<(String, String)>| {
            pairs.iter().any(|(ty, label)| {
                (*ty == item.match_ty || *ty == item.ty)
                    && literal_matches(text, label, item.composed)
            })
        };
        if hit_pair(&rec.activated_pairs) {
            return Reach::Activated;
        }
        if hit_pair(&rec.touched_pairs) {
            return Reach::Touched;
        }
        // An inline element that declares its role itself is also recognised by its label alone.
        if !item.site {
            if rec
                .activated_labels
                .iter()
                .any(|label| literal_matches(text, label, false))
            {
                return Reach::Activated;
            }
            if rec
                .touched_labels
                .iter()
                .any(|label| literal_matches(text, label, false))
            {
                return Reach::Touched;
            }
        }
    }
    Reach::Missed
}

/// The pieces of a string literal of the `.slint` source as the running element shows it: the
/// fixed text between the interpolations. An escape (`\u{2026}`, `\"`, `\\`, `\n`) is resolved;
/// a bare backslash is what the scanner leaves of an interpolation `\{expr}`, so it splits the
/// literal into pieces. One piece means no interpolation.
pub(super) fn literal_segments(raw: &str) -> Vec<String> {
    let mut segments: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            current.push(ch);
            continue;
        }
        match chars.peek().copied() {
            Some('u') => {
                chars.next();
                let mut digits = String::new();
                if chars.peek() == Some(&'{') {
                    chars.next();
                    for next in chars.by_ref() {
                        if next == '}' {
                            break;
                        }
                        digits.push(next);
                    }
                }
                if let Some(resolved) = u32::from_str_radix(&digits, 16)
                    .ok()
                    .and_then(char::from_u32)
                {
                    current.push(resolved);
                }
            }
            Some(escaped @ ('"' | '\\')) => {
                chars.next();
                current.push(escaped);
            }
            Some('n') => {
                chars.next();
                current.push('\n');
            }
            // An interpolation: the pieces before and after it are separate.
            _ => segments.push(std::mem::take(&mut current)),
        }
    }
    segments.push(current);
    segments
}

/// Whether the running label `actual` is the literal `raw` of a site. `composed`: the component
/// adds to the literal, so the label only has to contain it. A literal with interpolations needs
/// at least three characters of fixed text (a literal that is only interpolations and spaces
/// cannot be told from any other label); without `composed` the label must also start with the
/// first piece and end with the last.
pub(super) fn literal_matches(raw: &str, actual: &str, composed: bool) -> bool {
    let segments = literal_segments(raw);
    if let [text] = segments.as_slice() {
        return if composed {
            actual.contains(text.as_str())
        } else {
            actual == text
        };
    }
    let fixed: usize = segments.iter().map(|s| s.trim().chars().count()).sum();
    if fixed < 3 {
        return false;
    }
    let mut position = 0;
    for (index, segment) in segments.iter().enumerate() {
        if segment.is_empty() {
            continue;
        }
        let Some(found) = actual[position..].find(segment.as_str()) else {
            return false;
        };
        if index == 0 && !composed && found != 0 {
            return false;
        }
        position += found + segment.len();
    }
    let tail = segments.last().map_or("", String::as_str);
    composed || tail.is_empty() || actual.ends_with(tail)
}

#[cfg(test)]
mod tests;
