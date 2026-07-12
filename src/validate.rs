//! Invariant validation.
//!
//! Every load path calls `validate(&asset)` after `migrate(&asset)` and
//! before returning. A conforming `.beui` asset MUST satisfy all
//! invariants; the first violation produces a structured
//! [`ProtocolError::InvariantViolation`] with a stable `kind` string
//! for programmatic consumers.
//!
//! ## Invariant catalogue
//!
//! | kind string         | rule                                                  |
//! |---------------------|-------------------------------------------------------|
//! | `multiple_roots`    | At most one widget in the tree has `is_root: true`.   |
//! | `duplicate_id`      | All widget ids in the tree are unique.                |
//! | `missing_transform` | Every widget carries a `transform` component.         |
//! | `missing_style`     | Every widget carries a `style` component.             |
//! | `image_without_image`   | `kind: Image` widgets carry an `image` component. |
//! | `include_without_include` | `kind: Include` widgets carry an `include` component. |
//! | `text_without_text`     | `kind: Text` widgets carry a `text` component.     |
//! | `wrong_version`    | `asset.version == CURRENT_SCHEMA_VERSION`.            |
//!
//! Add new invariants by appending a new `kind` string here AND a
//! corresponding row in the catalogue above.

use crate::error::ProtocolError;
use crate::migrate::CURRENT_SCHEMA_VERSION;
use crate::schema::{UiDefinitionAsset, WidgetKind, WidgetNode};
use std::collections::HashSet;

/// Validate all invariants. Returns the first violation found, with a
/// stable `kind` string and a human-readable `detail`.
pub fn validate(asset: &UiDefinitionAsset) -> Result<(), ProtocolError> {
    if asset.version != CURRENT_SCHEMA_VERSION {
        return Err(ProtocolError::InvariantViolation {
            kind: "wrong_version",
            detail: format!(
                "asset version {} != current {}",
                asset.version, CURRENT_SCHEMA_VERSION
            ),
        });
    }
    let mut seen_ids: HashSet<&str> = HashSet::new();
    let mut root_count = 0;
    validate_node(&asset.root, &mut seen_ids, &mut root_count)?;
    if root_count > 1 {
        return Err(ProtocolError::InvariantViolation {
            kind: "multiple_roots",
            detail: format!("{root_count} widgets carry is_root: true (max 1)"),
        });
    }
    Ok(())
}

fn validate_node<'a>(
    node: &'a WidgetNode,
    seen_ids: &mut HashSet<&'a str>,
    root_count: &mut u32,
) -> Result<(), ProtocolError> {
    if !seen_ids.insert(node.id.as_str()) {
        return Err(ProtocolError::InvariantViolation {
            kind: "duplicate_id",
            detail: format!("widget id {:?} appears more than once in the tree", node.id),
        });
    }
    if node.is_root {
        *root_count += 1;
    }
    if !node.components.contains_key("transform") {
        return Err(ProtocolError::InvariantViolation {
            kind: "missing_transform",
            detail: format!("widget {:?} has no `transform` component", node.id),
        });
    }
    if !node.components.contains_key("style") {
        return Err(ProtocolError::InvariantViolation {
            kind: "missing_style",
            detail: format!("widget {:?} has no `style` component", node.id),
        });
    }
    match node.kind {
        WidgetKind::Image if !node.components.contains_key("image") => {
            return Err(ProtocolError::InvariantViolation {
                kind: "image_without_image",
                detail: format!("Image widget {:?} has no `image` component", node.id),
            });
        }
        WidgetKind::Include if !node.components.contains_key("include") => {
            return Err(ProtocolError::InvariantViolation {
                kind: "include_without_include",
                detail: format!("Include widget {:?} has no `include` component", node.id),
            });
        }
        WidgetKind::Text if !node.components.contains_key("text") => {
            return Err(ProtocolError::InvariantViolation {
                kind: "text_without_text",
                detail: format!("Text widget {:?} has no `text` component", node.id),
            });
        }
        _ => {}
    }
    for child in &node.children {
        validate_node(child, seen_ids, root_count)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{ComponentPayload, StyleProps, TransformProps, UiDefinitionAsset, WidgetKind};

    fn minimal_valid() -> UiDefinitionAsset {
        let mut root = WidgetNode {
            id: "root".into(),
            name: None,
            is_root: true,
            kind: WidgetKind::Container,
            children: vec![],
            components: Default::default(),
        };
        root.components.insert(
            "transform".into(),
            ComponentPayload::Transform(TransformProps::default()),
        );
        root.components.insert(
            "style".into(),
            ComponentPayload::Style(StyleProps::default()),
        );
        UiDefinitionAsset {
            version: CURRENT_SCHEMA_VERSION,
            root,
            theme: None,
        }
    }

    #[test]
    fn minimal_valid_passes() {
        assert!(validate(&minimal_valid()).is_ok());
    }

    #[test]
    fn rejects_wrong_version() {
        let mut a = minimal_valid();
        a.version = 2;
        let err = validate(&a).unwrap_err();
        assert_eq!(err.kind(), "invariant_violation");
        let detail = format!("{err}");
        assert!(detail.contains("wrong_version"));
    }

    #[test]
    fn rejects_duplicate_id() {
        let mut a = minimal_valid();
        let mut child = a.root.clone();
        child.id = "root".into(); // collide with parent
        child.is_root = false;
        a.root.children.push(child);
        let err = validate(&a).unwrap_err();
        let detail = format!("{err}");
        assert!(detail.contains("duplicate_id"));
    }

    #[test]
    fn rejects_multiple_roots() {
        let mut a = minimal_valid();
        let mut child = a.root.clone();
        child.id = "second-root".into();
        child.is_root = true;
        a.root.children.push(child);
        let err = validate(&a).unwrap_err();
        let detail = format!("{err}");
        assert!(detail.contains("multiple_roots"));
    }

    #[test]
    fn rejects_missing_transform() {
        let mut a = minimal_valid();
        a.root.components.remove("transform");
        let err = validate(&a).unwrap_err();
        let detail = format!("{err}");
        assert!(detail.contains("missing_transform"));
    }

    #[test]
    fn rejects_image_without_image_component() {
        let mut a = minimal_valid();
        a.root.kind = WidgetKind::Image;
        let err = validate(&a).unwrap_err();
        let detail = format!("{err}");
        assert!(detail.contains("image_without_image"));
    }

    #[test]
    fn rejects_include_without_include_component() {
        let mut a = minimal_valid();
        a.root.kind = WidgetKind::Include;
        let err = validate(&a).unwrap_err();
        let detail = format!("{err}");
        assert!(detail.contains("include_without_include"));
    }

    #[test]
    fn rejects_text_without_text_component() {
        let mut a = minimal_valid();
        a.root.kind = WidgetKind::Text;
        let err = validate(&a).unwrap_err();
        let detail = format!("{err}");
        assert!(detail.contains("text_without_text"));
    }
}