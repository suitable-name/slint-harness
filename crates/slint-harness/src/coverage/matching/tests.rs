//! Unit tests of the matching rules and of the parser rules they depend on (no GUI: the recorder
//! is filled by hand and the inventory is read from a source text).

use super::{Reach, literal_matches, reach_of, single_site_types};
use crate::coverage::{
    inventory::{ElementItem, Inventory, Label, inventory_of},
    props::LabelProps,
    recorder::Recorder,
};

/// A page with a single-site app component whose label is computed, a wrapper whose reader label
/// differs from its text, an element with an id behind an `if`, two literals on one line and a
/// literal with an interpolation.
const PAGE: &str = r#"
component Chip inherits Rectangle {
    in property <string> caption;
    accessible-role: button;
    accessible-label: root.caption;
    TouchArea { }
}
component Wrap inherits Rectangle {
    in property <string> text;
    in property <string> reader_label;
    Button {
        text: root.text;
        accessible-label: root.reader_label == "" ? root.text : root.reader_label;
    }
}
export component Page inherits Window {
    Chip { caption: root.cap; }
    Wrap { text: "Visible"; reader_label: "Spoken"; }
    if root.show: tip := Wrap { text: root.t; }
    LineEdit { placeholder-text: "Vol/W"; accessible-label: "Printed Volume"; }
    Button { text: "Layer \{root.n}"; }
}
"#;

/// A page whose only `Rectangle` site declares its role itself and has a computed label. The
/// second control keeps `Page` from being a wrapper (one interactive element whose label reads
/// `root.<property>` would make it one, and its inner element no entry).
const INLINE: &str = r#"
export component Page inherits Window {
    Rectangle { accessible-role: button; accessible-label: root.x; }
    Button { text: "Other"; }
}
"#;

fn reader(names: &[&str]) -> LabelProps {
    LabelProps {
        extra: Vec::new(),
        reader: names.iter().map(|n| (*n).to_owned()).collect(),
    }
}

fn site<'a>(inv: &'a Inventory, ty: &str) -> &'a ElementItem {
    inv.elements
        .iter()
        .find(|e| e.ty == ty)
        .unwrap_or_else(|| panic!("no {ty} site among {} elements", inv.elements.len()))
}

fn wrap_site(inv: &Inventory, id: Option<&str>) -> ElementItem {
    inv.elements
        .iter()
        .find(|e| e.ty == "Wrap" && e.id.as_deref() == id)
        .cloned()
        .unwrap_or_else(|| panic!("no Wrap site with id {id:?}"))
}

fn rec(fill: impl FnOnce(&mut Recorder)) -> Recorder {
    let mut recorder = Recorder::new();
    fill(&mut recorder);
    recorder
}

fn set(names: &[&str]) -> std::collections::BTreeSet<String> {
    names.iter().map(|n| (*n).to_owned()).collect()
}

fn pair(ty: &str, label: &str) -> (String, String) {
    (ty.to_owned(), label.to_owned())
}

#[test]
fn escapes_are_resolved_before_comparing() {
    assert!(literal_matches("Say \\\"hi\\\"", "Say \"hi\"", false));
    assert!(literal_matches("More\\u{2026}", "More\u{2026}", false));
    assert!(literal_matches("a\\nb", "a\nb", false));
    assert!(literal_matches("a\\\\b", "a\\b", false));
    assert!(!literal_matches("Say \\\"hi\\\"", "Say hi", false));
}

#[test]
fn an_interpolated_literal_matches_by_its_fixed_pieces() {
    // The scanner leaves a bare backslash where `\{expr}` was.
    assert!(literal_matches("Layer \\", "Layer 3", false));
    assert!(literal_matches("Count \\ items", "Count 4 items", false));
    assert!(!literal_matches("Count \\ items", "Count 4 things", false));
    // Anchored at the start unless the component composes the label.
    assert!(!literal_matches("Layer \\", "xLayer 3", false));
    assert!(literal_matches("Layer \\", "xLayer 3", true));
    assert!(literal_matches(
        "Count \\ items",
        "Total Count 4 items left",
        true
    ));
    assert!(!literal_matches(
        "Count \\ items",
        "Total Count 4 things",
        true
    ));
}

#[test]
fn an_interpolated_literal_needs_three_fixed_characters() {
    assert!(!literal_matches("\\ \\", "x y", false));
    assert!(!literal_matches("A\\", "A1", false));
    assert!(literal_matches("Abc\\", "Abc1", false));
}

#[test]
fn a_plain_literal_is_exact_or_contained_when_composed() {
    assert!(literal_matches("Save", "Save", false));
    assert!(!literal_matches("Save", "Save all", false));
    assert!(literal_matches("Save", "Save all", true));
}

#[test]
fn an_id_behind_an_if_prefix_and_a_literal_per_statement_are_inventoried() {
    let inv = inventory_of("page.slint", PAGE, &LabelProps::default());
    let tip = wrap_site(&inv, Some("tip"));
    assert_eq!(tip.runtime_id().as_deref(), Some("Page::tip"));
    assert!(tip.trackable());
    let edit = site(&inv, "LineEdit");
    assert_eq!(edit.label, Label::Literal("Printed Volume".to_owned()));
    let layer = site(&inv, "Button");
    assert_eq!(layer.label, Label::Literal("Layer \\".to_owned()));
    assert!(inv.component_names().contains("Chip"));
    assert!(!inv.component_names().contains("Rectangle"));
}

#[test]
fn reader_props_decide_which_literal_a_wrapper_site_is_keyed_by() {
    let plain = inventory_of("page.slint", PAGE, &LabelProps::default());
    assert_eq!(
        wrap_site(&plain, None).label,
        Label::Literal("Visible".to_owned()),
        "with no configuration the text is the label"
    );
    let label_only = LabelProps {
        extra: vec!["reader_label".to_owned()],
        reader: Vec::new(),
    };
    let inv = inventory_of("page.slint", PAGE, &label_only);
    assert_eq!(
        wrap_site(&inv, None).label,
        Label::Literal("Visible".to_owned()),
        "a label prop alone does not beat the text"
    );
    let inv = inventory_of("page.slint", PAGE, &reader(&["reader_label"]));
    let wrap = wrap_site(&inv, None);
    assert_eq!(wrap.label, Label::Literal("Spoken".to_owned()));
    assert_eq!(wrap.key(), "page.slint|Page|\"Spoken\"");
}

#[test]
fn a_wrapper_site_is_credited_through_its_own_type_name() {
    let inv = inventory_of("page.slint", PAGE, &reader(&["reader_label"]));
    let wrap = wrap_site(&inv, None);
    let single = single_site_types(&inv);
    // `Wrap` is the site's type, `Button` the running inner element's.
    let by_site_type = rec(|r| {
        r.activated_pairs.insert(pair("Wrap", "Spoken"));
    });
    assert_eq!(reach_of(&wrap, &by_site_type, &single), Reach::Activated);
    let by_inner_type = rec(|r| {
        r.touched_pairs.insert(pair("Button", "Spoken"));
    });
    assert_eq!(reach_of(&wrap, &by_inner_type, &single), Reach::Touched);
    let wrong_label = rec(|r| {
        r.activated_pairs.insert(pair("Button", "Visible"));
    });
    assert_eq!(reach_of(&wrap, &wrong_label, &single), Reach::Missed);
}

#[test]
fn an_interpolated_site_literal_is_credited_by_the_running_label() {
    let inv = inventory_of("page.slint", PAGE, &LabelProps::default());
    let layer = site(&inv, "Button");
    let single = single_site_types(&inv);
    let hit = rec(|r| {
        r.activated_pairs.insert(pair("Button", "Layer 4"));
    });
    assert_eq!(reach_of(layer, &hit, &single), Reach::Activated);
    let miss = rec(|r| {
        r.activated_pairs.insert(pair("Button", "Other 4"));
    });
    assert_eq!(reach_of(layer, &miss, &single), Reach::Missed);
}

#[test]
fn the_site_id_credits_an_element_with_a_computed_label() {
    let inv = inventory_of("page.slint", PAGE, &LabelProps::default());
    let tip = wrap_site(&inv, Some("tip"));
    let single = single_site_types(&inv);
    assert_eq!(reach_of(&tip, &Recorder::new(), &single), Reach::Missed);
    let by_id = rec(|r| {
        r.touched_ids.insert("Page::tip".to_owned());
    });
    assert_eq!(reach_of(&tip, &by_id, &single), Reach::Touched);
}

#[test]
fn the_one_site_of_an_app_component_is_credited_by_type_or_by_base() {
    let inv = inventory_of("page.slint", PAGE, &LabelProps::default());
    let chip = site(&inv, "Chip");
    assert_eq!(chip.label, Label::None);
    let single = single_site_types(&inv);
    assert_eq!(reach_of(chip, &Recorder::new(), &single), Reach::Missed);
    let by_type = rec(|r| {
        r.activated_types.extend(set(&["Chip"]));
    });
    assert_eq!(reach_of(chip, &by_type, &single), Reach::Activated);
    let by_base = rec(|r| {
        r.touched_bases.extend(set(&["Rectangle", "Chip"]));
    });
    assert_eq!(reach_of(chip, &by_base, &single), Reach::Touched);
    let builtin_only = rec(|r| {
        r.activated_bases.extend(set(&["Rectangle", "TouchArea"]));
    });
    assert_eq!(reach_of(chip, &builtin_only, &single), Reach::Missed);
}

#[test]
fn a_builtin_is_credited_by_type_but_never_by_base() {
    let inv = inventory_of("inline.slint", INLINE, &LabelProps::default());
    let rectangle = site(&inv, "Rectangle");
    let single = single_site_types(&inv);
    let by_base = rec(|r| {
        r.activated_bases.extend(set(&["Rectangle"]));
    });
    assert_eq!(reach_of(rectangle, &by_base, &single), Reach::Missed);
    let by_type = rec(|r| {
        r.activated_types.extend(set(&["Rectangle"]));
    });
    assert_eq!(reach_of(rectangle, &by_type, &single), Reach::Activated);
}

#[test]
fn a_type_with_several_sites_is_never_credited_by_type() {
    let inv = inventory_of("page.slint", PAGE, &LabelProps::default());
    let tip = wrap_site(&inv, Some("tip"));
    let single = single_site_types(&inv);
    // Three sites report `Button` (two wrappers and the literal one): no credit from the type.
    let by_type = rec(|r| {
        r.activated_types.extend(set(&["Button"]));
    });
    assert_eq!(reach_of(&tip, &by_type, &single), Reach::Missed);
}
