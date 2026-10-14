//! Statement helpers of the inventory scanner: what a masked `.slint` statement declares (the
//! statement helpers of Indicatrix `tests/ui_static.rs`). Pure text functions, no state.

/// Whether `ch` can be part of a Slint identifier (`-` counts: `tip-button`).
pub(super) fn is_ident_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '-'
}

/// `(is_global, name, base)` of a `[export] component|global Name [inherits Base]` header.
pub(super) fn component_header(stmt: &str) -> Option<(bool, String, Option<String>)> {
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
pub(super) fn element_type(stmt: &str) -> Option<String> {
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
pub(super) fn handler_name(stmt: &str) -> Option<String> {
    let head = stmt.trim().trim_end_matches("=>").trim();
    let name: String = head.chars().take_while(|c| is_ident_char(*c)).collect();
    let rest = head[name.len()..].trim();
    (!name.is_empty() && (rest.is_empty() || rest.starts_with('('))).then_some(name)
}

/// The name of a `[pure|public] callback name(...)...;` statement.
pub(super) fn parse_callback(stmt: &str) -> Option<String> {
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
pub(super) fn property_value<'a>(stmt: &'a str, property: &str) -> Option<&'a str> {
    let rest = stmt.trim().strip_prefix(property)?.trim_start();
    rest.strip_prefix(':').map(str::trim)
}

/// The id an element statement declares: the text before `:=` after the last `:` of an
/// `if cond:` / `for x in y:` prefix (for example, in Indicatrix,
/// `if root.advanced: tip-button-ranges-open := TipButton` has the id `tip-button-ranges-open`,
/// which is what the running element reports).
pub(super) fn element_id(stmt: &str) -> Option<String> {
    stmt.split_once(":=")
        .map(|(id, _)| id.rsplit(':').next().unwrap_or(id).trim().to_owned())
        .filter(|id| !id.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{element_id, element_type, property_value};

    #[test]
    fn the_id_after_an_if_or_for_prefix_is_the_element_id() {
        assert_eq!(element_id("ok := Button").as_deref(), Some("ok"));
        assert_eq!(
            element_id("if root.advanced: tip-button := TipButton").as_deref(),
            Some("tip-button")
        );
        assert_eq!(
            element_id("for row in rows: cell := Cell").as_deref(),
            Some("cell")
        );
        assert_eq!(element_id("if root.advanced: Button"), None);
        assert_eq!(element_id("Button"), None);
        assert_eq!(
            element_type("if root.advanced: tip-button := TipButton").as_deref(),
            Some("TipButton")
        );
    }

    #[test]
    fn a_property_prefix_of_another_name_is_not_the_property() {
        assert_eq!(property_value("text: root.x", "text"), Some("root.x"));
        assert_eq!(property_value("text-color: red", "text"), None);
    }
}
