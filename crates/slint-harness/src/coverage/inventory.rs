//! The static inventory of the coverage gate: global callbacks with a Rust handler and interactive
//! elements, read from the `.slint` files under the scan root (see the parent module for how the
//! run is matched against it).
//!
//! What counts:
//!
//! * **callbacks**: every `callback` declared in a `global` that has no Slint body (no
//!   `name => { ... }` handler and no `name <=> other` forwarding), as `Model.name` in snake
//!   case;
//! * **interactive elements**: every INSTANTIATION SITE of an interactive type, plus every inline
//!   element that declares an interactive `accessible-role` itself (button, checkbox, switch,
//!   combobox, slider, spinbox, tab, text-input). Interactive types are the std widgets
//!   (`Button`, `CheckBox`, `ComboBox`, `LineEdit` ...), the app components whose root declares
//!   such a role, WRAPPER components (exactly one interactive element inside whose label reads
//!   `root.<property>`, for example, in Indicatrix, `TipButton`) and everything that inherits one
//!   of those. A component's own root or wrapped inner element is not an entry: its uses are.
//!   One entry per site, so a site in a `for` template counts once.
//!
//! Known limits of the text scanner (plan §1.6; it produced 503 callbacks and 907 elements on
//! Indicatrix, 135 of the elements untrackable):
//!
//! * a component type must start with a capital letter to be recognised as an element;
//! * `import { X as Y }` renames are not followed;
//! * `@children` slots and `states` blocks are opaque;
//! * a label built by an expression cannot be tracked (the element is listed as "untrackable"
//!   unless it has an `id :=`).
//!
//! Replace the whole file by a compiler-based source when a second app needs one; the rest of
//! the gate only sees [`Inventory`].

use super::{
    recorder::normalise_id,
    scan::{Event, Kind, events, scan},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

/// `accessible-role` values that make an element interactive.
const INTERACTIVE_ROLES: &[&str] = &[
    "button",
    "checkbox",
    "switch",
    "combobox",
    "slider",
    "spinbox",
    "tab",
    "text-input",
];

/// A global callback that has a Rust handler (no Slint body).
#[derive(Debug, Clone)]
pub struct CallbackItem {
    /// `Model.name`, name in snake case.
    pub key: String,
    /// Source file relative to the scan root.
    pub file: String,
    /// Declaration line.
    pub line: usize,
}

/// How an element names its accessible label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Label {
    /// A string literal (`"Save"`, `@tr("Save")`).
    Literal(String),
    /// Any other expression (`root.text`).
    Expression(String),
    /// No `accessible-label` in the block.
    None,
}

/// An interactive element: an instantiation site of an interactive type, or an inline element
/// that declares an interactive `accessible-role` itself.
#[derive(Debug, Clone)]
pub struct ElementItem {
    /// Source file relative to the scan root.
    pub file: String,
    /// Line of the element's block.
    pub line: usize,
    /// Component that contains it.
    pub component: String,
    /// `id :=` of the element.
    pub id: Option<String>,
    /// Whether this is an instantiation site of an interactive type (else an inline element that
    /// declares its role itself, which the label alone also recognises).
    pub site: bool,
    /// Element type at the site (`Button`, `TipButton`, `TouchArea` ...).
    pub ty: String,
    /// The type the running element reports (differs from `ty` for a wrapper component, whose
    /// interactive element is an inner `Button`).
    pub match_ty: String,
    /// The role (`button` ...).
    pub role: String,
    /// The label the site gives the element.
    pub label: Label,
    /// Whether the running label only CONTAINS the site's label (the component adds to it).
    pub composed: bool,
}

impl ElementItem {
    /// The allow-list key: `file|Component|id`, `file|Component|"label"` or
    /// `file|Component|<Type@line>`.
    #[must_use]
    pub fn key(&self) -> String {
        let tail = match (&self.id, &self.label) {
            (Some(id), _) => id.clone(),
            (None, Label::Literal(text)) => format!("\"{text}\""),
            _ => format!("<{}@{}>", self.ty, self.line),
        };
        format!("{}|{}|{}", self.file, self.component, tail)
    }

    /// The runtime id this element would have (`Component::id`), when it has an id.
    pub(super) fn runtime_id(&self) -> Option<String> {
        self.id
            .as_ref()
            .map(|id| normalise_id(&format!("{}::{id}", self.component)))
    }

    /// Whether the run can ever recognise this element.
    #[must_use]
    pub fn trackable(&self) -> bool {
        self.id.is_some() || matches!(self.label, Label::Literal(_))
    }

    pub(super) fn describe(&self) -> String {
        let label = match &self.label {
            Label::Literal(text) => format!(" \"{text}\""),
            Label::Expression(expr) => format!(" label={expr}"),
            Label::None => String::new(),
        };
        format!(
            "{}:{} {} {}{label} [{}]",
            self.file,
            self.line,
            self.role,
            self.ty,
            self.key()
        )
    }
}

/// An interactive type: what its instantiation sites are recorded with.
#[derive(Debug, Clone)]
struct TypeInfo {
    role: String,
    /// A std widget (its label is the site's `accessible-label`, `text` or `title`).
    std: bool,
    /// The site property the running label shows (`text` for `root.text`).
    prop: Option<String>,
    /// The running label is more than that property alone.
    composed: bool,
    /// The element type the running element reports.
    match_ty: String,
}

/// A component whose root declares an interactive role (pass 1).
struct RootRecord {
    component: String,
    role: String,
    label: Option<Label>,
}

/// An interactive element inside a component, with its label (pass 1; wrapper detection).
struct InnerRecord {
    file: String,
    line: usize,
    component: String,
    ty: String,
    role: String,
    label: Option<Label>,
}

/// Everything the static scan found.
#[derive(Default)]
pub struct Inventory {
    /// Global callbacks with a Rust handler.
    pub callbacks: Vec<CallbackItem>,
    /// Interactive elements.
    pub elements: Vec<ElementItem>,
    roots: Vec<RootRecord>,
    inners: Vec<InnerRecord>,
    /// `(component, base)` of every component.
    components: Vec<(String, Option<String>)>,
}

/// The interactive types and the inner elements their wrapper sites replace.
struct TypeTable {
    types: BTreeMap<String, TypeInfo>,
    wrapped: BTreeSet<(String, usize)>,
}

/// std-widgets types and the role their root carries.
const STD_WIDGETS: &[(&str, &str)] = &[
    ("Button", "button"),
    ("StandardButton", "button"),
    ("CheckBox", "checkbox"),
    ("Switch", "switch"),
    ("ComboBox", "combobox"),
    ("Slider", "slider"),
    ("SpinBox", "spinbox"),
    ("LineEdit", "text-input"),
    ("TextEdit", "text-input"),
    ("Tab", "tab"),
    ("StandardListView", "list"),
];

/// Properties read as an element's label.
const LABEL_PROPS: &[&str] = &["accessible-label", "text", "label", "title", "hint", "name"];
/// Which `root.<property>` of a label expression is the visible text, in order of preference.
const PREFERRED_PROPS: &[&str] = &["text", "label", "title", "name", "hint"];
/// Where a std widget's label comes from.
const STD_LABEL_ORDER: &[&str] = &["accessible-label", "text", "title"];

// ---- parsing helpers (the statement helpers of Indicatrix tests/ui_static.rs) ----

fn is_ident_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '-'
}

/// `(is_global, name, base)` of a `[export] component|global Name [inherits Base]` header.
fn component_header(stmt: &str) -> Option<(bool, String, Option<String>)> {
    let words: Vec<&str> = stmt.split_whitespace().collect();
    let at = words
        .iter()
        .position(|w| *w == "component" || *w == "global")?;
    let name = (*words.get(at + 1)?).to_owned();
    let base = (words.get(at + 2) == Some(&"inherits"))
        .then(|| words.get(at + 3).map(|w| (*w).to_owned()))
        .flatten();
    Some((words[at] == "global", name, base))
}

/// The type of an element statement (`Type`, `id := Type`, `if c: Type`, `for x in y: Type`).
fn element_type(stmt: &str) -> Option<String> {
    let s = stmt.trim();
    let tail = if let Some((_, after)) = s.split_once(":=") {
        after
    } else if s.starts_with("if ") || s.starts_with("for ") {
        s.rsplit_once(':')?.1
    } else {
        s
    };
    let word: String = tail
        .trim()
        .chars()
        .take_while(|c| is_ident_char(*c) || *c == '.')
        .collect();
    word.chars().next()?.is_uppercase().then_some(word)
}

/// The callback name of a handler header such as `clicked =>` or `edited(text) =>`.
fn handler_name(stmt: &str) -> Option<String> {
    let head = stmt.trim().trim_end_matches("=>").trim();
    let name: String = head.chars().take_while(|c| is_ident_char(*c)).collect();
    let rest = head[name.len()..].trim();
    (!name.is_empty() && (rest.is_empty() || rest.starts_with('('))).then_some(name)
}

/// The name of a `[pure|public] callback name(...)...;` statement.
fn parse_callback(stmt: &str) -> Option<String> {
    let mut s = stmt.trim();
    while let Some(rest) = s
        .strip_prefix("pure ")
        .or_else(|| s.strip_prefix("public "))
        .or_else(|| s.strip_prefix("private "))
    {
        s = rest.trim_start();
    }
    let rest = s.strip_prefix("callback ")?.trim();
    let name: String = rest.chars().take_while(|c| is_ident_char(*c)).collect();
    (!name.is_empty()).then_some(name)
}

/// The value of `property: value` when the statement sets `property`.
fn property_value<'a>(stmt: &'a str, property: &str) -> Option<&'a str> {
    let rest = stmt.trim().strip_prefix(property)?.trim_start();
    rest.strip_prefix(':').map(str::trim)
}

/// What an element block collects while it is open.
#[derive(Default, Clone)]
struct ElementFrame {
    ty: String,
    id: Option<String>,
    line: usize,
    role: Option<String>,
    /// The label-like properties set in the block, by property name.
    props: BTreeMap<String, Label>,
}

enum Frame {
    /// A `global` body.
    Global { name: String },
    /// A `component` body, with its own root element.
    Component { name: String, root: ElementFrame },
    /// An element inside a component.
    Element(ElementFrame),
    /// A callback handler body.
    Handler,
    /// Anything else.
    Other,
}

/// Per-file scan state.
struct FileScan<'a> {
    file: &'a str,
    strings_by_line: BTreeMap<usize, Vec<&'a str>>,
    stack: Vec<Frame>,
    inventory: &'a mut Inventory,
    /// `None` in pass 1 (collecting roots and inner elements), the interactive types in pass 2.
    table: Option<&'a TypeTable>,
    /// `(global, callback, line)` declarations and the `(global, callback)` pairs with a body.
    declared: Vec<(String, String, usize)>,
    bodies: BTreeSet<(String, String)>,
}

impl FileScan<'_> {
    fn owner(&self) -> Option<&str> {
        self.stack.iter().rev().find_map(|f| match f {
            Frame::Component { name, .. } => Some(name.as_str()),
            _ => None,
        })
    }

    fn open(&mut self, ev: &Event) {
        let text = ev.text.trim();
        if text.ends_with("=>") {
            if let (Some(Frame::Global { name }), Some(callback)) =
                (self.stack.last(), handler_name(text))
            {
                self.bodies.insert((name.clone(), callback));
            }
            self.stack.push(Frame::Handler);
            return;
        }
        if self.stack.is_empty() {
            let header = component_header(text);
            if let (None, Some((false, name, base))) = (self.table, &header) {
                self.inventory.components.push((name.clone(), base.clone()));
            }
            let frame = match header {
                Some((true, name, _)) => Frame::Global { name },
                Some((false, name, base)) => Frame::Component {
                    root: ElementFrame {
                        ty: base.unwrap_or_else(|| name.clone()),
                        id: Some("root".to_owned()),
                        line: ev.line,
                        ..ElementFrame::default()
                    },
                    name,
                },
                None => Frame::Other,
            };
            self.stack.push(frame);
            return;
        }
        let parent_holds_elements = matches!(
            self.stack.last(),
            Some(Frame::Component { .. } | Frame::Element(_))
        );
        let frame = match (parent_holds_elements, element_type(text)) {
            (true, Some(ty)) => Frame::Element(ElementFrame {
                ty,
                id: text
                    .split_once(":=")
                    .map(|(id, _)| id.trim().to_owned())
                    .filter(|id| !id.is_empty()),
                line: ev.line,
                ..ElementFrame::default()
            }),
            _ => Frame::Other,
        };
        self.stack.push(frame);
    }

    fn statement(&mut self, ev: &Event) {
        let text = ev.text.trim();
        let label = LABEL_PROPS.iter().find_map(|name| {
            property_value(text, name).map(|v| ((*name).to_owned(), self.label_of(v, ev.line)))
        });
        match self.stack.last_mut() {
            Some(Frame::Global { name }) => {
                if let Some(callback) = parse_callback(text) {
                    self.declared.push((name.clone(), callback, ev.line));
                } else if let Some((left, _)) = text.split_once("<=>") {
                    let left = left.trim();
                    if !left.is_empty() && left.chars().all(is_ident_char) {
                        self.bodies.insert((name.clone(), left.to_owned()));
                    }
                }
            }
            Some(Frame::Component { root: frame, .. } | Frame::Element(frame)) => {
                if let Some(value) = property_value(text, "accessible-role") {
                    let role = value.strip_prefix("AccessibleRole.").unwrap_or(value);
                    frame.role = Some(role.trim().to_owned());
                } else if let Some((name, label)) = label {
                    frame.props.insert(name, label);
                }
            }
            _ => {}
        }
    }

    /// Classifies the right-hand side of `accessible-label:` (masked text) on `line`.
    fn label_of(&self, masked_value: &str, line: usize) -> Label {
        let compact: String = masked_value
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        let inner = compact
            .strip_prefix("@tr(")
            .and_then(|s| s.strip_suffix(')'))
            .unwrap_or(&compact);
        let blank_literal = inner.len() >= 2
            && inner.starts_with('"')
            && inner.ends_with('"')
            && inner[1..inner.len() - 1].is_empty();
        if blank_literal {
            let text = self
                .strings_by_line
                .get(&line)
                .and_then(|strings| strings.first())
                .copied()
                .unwrap_or_default();
            Label::Literal(text.to_owned())
        } else {
            Label::Expression(
                masked_value
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        }
    }

    fn close(&mut self) {
        let Some(frame) = self.stack.pop() else {
            return;
        };
        match frame {
            Frame::Component { name, root } => self.close_root(&name, &root),
            Frame::Element(element) => {
                let owner = self.owner().unwrap_or("?").to_owned();
                self.close_element(&owner, element);
            }
            Frame::Global { name } => {
                // Resolve the global callbacks now that every body was seen.
                let declared = std::mem::take(&mut self.declared);
                for (global, callback, line) in declared {
                    if global == name && !self.bodies.contains(&(global.clone(), callback.clone()))
                    {
                        self.inventory.callbacks.push(CallbackItem {
                            key: format!("{global}.{}", callback.replace('-', "_")),
                            file: self.file.to_owned(),
                            line,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    /// A component root: pass 1 remembers an interactive one (its uses are the entries).
    fn close_root(&mut self, name: &str, root: &ElementFrame) {
        let Some(role) = root
            .role
            .as_deref()
            .filter(|r| INTERACTIVE_ROLES.contains(r))
        else {
            return;
        };
        if self.table.is_none() {
            self.inventory.roots.push(RootRecord {
                component: name.to_owned(),
                role: role.to_owned(),
                label: root.props.get("accessible-label").cloned(),
            });
        }
    }

    fn close_element(&mut self, component: &str, element: ElementFrame) {
        let declared = element
            .role
            .clone()
            .filter(|r| INTERACTIVE_ROLES.contains(&r.as_str()));
        let Some(table) = self.table else {
            // Pass 1: an interactive element inside a component may make it a wrapper.
            let std_role = STD_WIDGETS
                .iter()
                .find(|(ty, _)| *ty == element.ty)
                .map(|(_, role)| (*role).to_owned());
            if let Some(role) = declared.or(std_role) {
                let label = STD_LABEL_ORDER
                    .iter()
                    .find_map(|p| element.props.get(*p))
                    .cloned();
                self.inventory.inners.push(InnerRecord {
                    file: self.file.to_owned(),
                    line: element.line,
                    component: component.to_owned(),
                    ty: element.ty,
                    role,
                    label,
                });
            }
            return;
        };
        if table
            .wrapped
            .contains(&(self.file.to_owned(), element.line))
        {
            return;
        }
        let (role, label, site, composed, match_ty) = if let Some(role) = declared {
            let label = element
                .props
                .get("accessible-label")
                .cloned()
                .unwrap_or(Label::None);
            (role, label, false, false, element.ty.clone())
        } else if let Some(info) = table.types.get(&element.ty) {
            let label = if info.std {
                STD_LABEL_ORDER
                    .iter()
                    .find_map(|p| element.props.get(*p))
                    .cloned()
            } else {
                element.props.get("accessible-label").cloned().or_else(|| {
                    info.prop
                        .as_ref()
                        .and_then(|prop| element.props.get(prop).cloned())
                })
            };
            (
                info.role.clone(),
                label.unwrap_or(Label::None),
                true,
                info.composed,
                info.match_ty.clone(),
            )
        } else {
            return;
        };
        self.inventory.elements.push(ElementItem {
            file: self.file.to_owned(),
            line: element.line,
            component: component.to_owned(),
            id: element.id,
            site,
            ty: element.ty,
            match_ty,
            role,
            label,
            composed,
        });
    }
}

/// Feeds one `.slint` file into `inventory`.
fn scan_file(file: &str, source: &str, inventory: &mut Inventory, table: Option<&TypeTable>) {
    let scanned = scan(source);
    let mut strings_by_line: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
    for string in &scanned.strings {
        strings_by_line
            .entry(string.line)
            .or_default()
            .push(string.text.as_str());
    }
    let mut state = FileScan {
        file,
        strings_by_line,
        stack: Vec::new(),
        inventory,
        table,
        declared: Vec::new(),
        bodies: BTreeSet::new(),
    };
    for ev in events(&scanned.masked) {
        match ev.kind {
            Kind::Open => state.open(&ev),
            Kind::Close => state.close(),
            Kind::Semi => state.statement(&ev),
        }
    }
}

fn collect_slint(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_slint(&path, out);
        } else if path.extension().is_some_and(|e| e == "slint") {
            out.push(path);
        }
    }
}

/// The label expression `expr` shows through `root.<property>`: the property, and whether the
/// expression is more than that property alone.
fn label_property(label: Option<&Label>) -> (Option<String>, bool) {
    let Some(Label::Expression(expr)) = label else {
        return (None, true);
    };
    let found: Vec<&str> = expr
        .match_indices("root.")
        .filter_map(|(at, _)| {
            let rest = &expr[at + "root.".len()..];
            let end = rest.find(|c: char| !is_ident_char(c)).unwrap_or(rest.len());
            (end > 0).then(|| &rest[..end])
        })
        .collect();
    let prop = PREFERRED_PROPS
        .iter()
        .find(|want| found.contains(want))
        .copied()
        .or_else(|| found.first().copied());
    match prop {
        Some(prop) => (Some(prop.to_owned()), expr.trim() != format!("root.{prop}")),
        None => (None, true),
    }
}

/// Builds the interactive types from pass 1: std widgets, components with an interactive root,
/// wrapper components (exactly one interactive element inside whose label reads `root.<prop>`)
/// and everything that inherits one of those.
fn type_table(pass1: &Inventory) -> TypeTable {
    let mut types: BTreeMap<String, TypeInfo> = STD_WIDGETS
        .iter()
        .map(|(ty, role)| {
            (
                (*ty).to_owned(),
                TypeInfo {
                    role: (*role).to_owned(),
                    std: true,
                    prop: None,
                    composed: false,
                    match_ty: (*ty).to_owned(),
                },
            )
        })
        .collect();
    for root in &pass1.roots {
        let (prop, composed) = label_property(root.label.as_ref());
        types.insert(
            root.component.clone(),
            TypeInfo {
                role: root.role.clone(),
                std: false,
                prop,
                composed,
                match_ty: root.component.clone(),
            },
        );
    }
    let mut inner_by_component: BTreeMap<&str, Vec<&InnerRecord>> = BTreeMap::new();
    for inner in &pass1.inners {
        inner_by_component
            .entry(inner.component.as_str())
            .or_default()
            .push(inner);
    }
    let mut wrapped = BTreeSet::new();
    for (component, inners) in inner_by_component {
        let [inner] = inners.as_slice() else {
            continue;
        };
        if types.contains_key(component) {
            continue;
        }
        let (prop, composed) = label_property(inner.label.as_ref());
        if prop.is_some() {
            types.insert(
                component.to_owned(),
                TypeInfo {
                    role: inner.role.clone(),
                    std: false,
                    prop,
                    composed,
                    match_ty: inner.ty.clone(),
                },
            );
            wrapped.insert((inner.file.clone(), inner.line));
        }
    }
    // Inheritors of an interactive type are interactive too.
    loop {
        let mut changed = false;
        for (name, base) in &pass1.components {
            let Some(base) = base else {
                continue;
            };
            if !types.contains_key(name) {
                if let Some(info) = types.get(base).cloned() {
                    types.insert(name.clone(), info);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    TypeTable { types, wrapped }
}

/// Reads `root/**/*.slint` (files sorted by path; `file` fields are relative to `root` with `/`
/// separators). A missing or empty `root` yields an empty inventory: the caller reports it.
#[must_use]
pub fn load_inventory(root: &Path) -> Inventory {
    let mut paths = Vec::new();
    collect_slint(root, &mut paths);
    let files: Vec<(String, String)> = paths
        .iter()
        .filter_map(|path| {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            fs::read_to_string(path).ok().map(|text| (rel, text))
        })
        .collect();
    let mut pass1 = Inventory::default();
    for (rel, text) in &files {
        scan_file(rel, text, &mut pass1, None);
    }
    let table = type_table(&pass1);
    let mut inventory = Inventory::default();
    for (rel, text) in &files {
        scan_file(rel, text, &mut inventory, Some(&table));
    }
    inventory
}

#[cfg(test)]
mod tests {
    use super::{Inventory, Label, scan_file, type_table};

    const SAMPLE: &str = r#"
export global Demo {
    callback handled(string);
    callback orphan();
    callback with-body() -> int;
    with-body => { return 1; }
}
export component Page inherits Window {
    ok := Button { text: "OK"; }
    Rectangle { accessible-role: AccessibleRole.checkbox; accessible-label: "Agree"; }
}
"#;

    #[test]
    fn callbacks_without_a_slint_body_and_interactive_elements_are_inventoried() {
        let mut pass1 = Inventory::default();
        scan_file("sample.slint", SAMPLE, &mut pass1, None);
        let table = type_table(&pass1);
        let mut inv = Inventory::default();
        scan_file("sample.slint", SAMPLE, &mut inv, Some(&table));
        let keys: Vec<&str> = inv.callbacks.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, ["Demo.handled", "Demo.orphan"]);
        assert_eq!(inv.elements.len(), 2);
        assert_eq!(inv.elements[0].key(), "sample.slint|Page|ok");
        assert_eq!(inv.elements[1].label, Label::Literal("Agree".to_owned()));
    }
}
