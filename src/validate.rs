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
use serde::Serialize;
use std::collections::HashSet;

/// A single auto-fix applied to an asset. Returned by `auto_fix` so
/// callers (the editor save pipeline, scripts) can surface what
/// changed to the user. The `kind` string is a stable identifier
/// (one per fixable rule, prefixed by the file/section); `summary` is
/// human-readable and may include the offending node id.
///
/// Designed to be `Serialize + Deserialize` so the Tauri `write_asset`
/// command can hand the list back to the editor over IPC.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ValidationFix {
    /// Stable, programmatic identifier for the rule that fired (e.g.
    /// `"root_flag_normalize"`). One string per fixable rule; safe to
    /// match on in the editor.
    pub kind: String,
    /// Human-readable description of what changed, may include the
    /// offending node id. Surfaced verbatim in the editor UI.
    pub summary: String,
}

/// Run auto-fixable invariant rules against the asset. Each rule
/// inspects the asset, optionally mutates it, and (if it acted) emits
/// a `ValidationFix` describing what it did. The list preserves
/// rule-application order so the editor can show the fixes in the
/// order they were applied.
///
/// Rules here are intentionally confined to SILENT, low-risk
/// corrections — the kind a user pressed Save and would have wanted
/// fixed anyway. Invariants that need human judgment (e.g.
/// `duplicate_id`, `multiple_roots`) stay in `validate()` and reject
/// the save with a structured `InvariantViolation`.
pub fn auto_fix(asset: &mut UiDefinitionAsset) -> Vec<ValidationFix> {
    let mut fixes = Vec::new();

    // Rule 1 — Root flag normalization. Per the design spec (see
    // docs/superpowers/specs/2026-07-11-root-container-and-artboard-
    // sizing-design.md), exactly one node per asset carries
    // `is_root: true` — the asset root container. The check in
    // `validate()` catches `multiple_roots` (>1 carries the flag),
    // but the orthogonal "zero nodes carry it" case (e.g. a
    // hand-migrated v1 fixture, or any file the editor loaded before
    // the projection-side normalization landed) is also a violation.
    // The save-time fix is unambiguous: the asset root by tree
    // position IS the root, so flip its flag and tell the user.
    //
    // We deliberately do NOT walk descendants here: even if a
    // descendant carries `is_root: true`, the `validate()` pass
    // afterward would surface `multiple_roots` and refuse the save —
    // auto-fixing a non-root descendant's flag would silently pick
    // the wrong "primary root" if a future schema evolves to support
    // sub-roots. Better to fail loud than to guess.
    if !asset.root.is_root {
        asset.root.is_root = true;
        fixes.push(ValidationFix {
            kind: "root_flag_normalize".into(),
            summary: format!(
                "Set `is_root: true` on the asset root ({}).",
                asset.root.id
            ),
        });
    }

    fixes
}

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

    /// Asset whose root has `is_root: false`. Used to exercise the
    /// auto-fix path; mirrors the on-disk shape seen in the
    /// hand-migrated `bevy-lab/ui-workspace/main-menu/scene.beui`
    /// fixture before 2026-07-17.
    fn root_flag_false() -> UiDefinitionAsset {
        let mut a = minimal_valid();
        a.root.is_root = false;
        a
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

    // -------------------------------------------------------------------
    // auto_fix tests
    // -------------------------------------------------------------------

    #[test]
    fn auto_fix_clearly_valid_asset_emits_no_fixes() {
        let mut a = minimal_valid();
        let fixes = auto_fix(&mut a);
        assert!(fixes.is_empty(), "expected no fixes, got {fixes:?}");
        assert!(a.root.is_root);
    }

    #[test]
    fn auto_fix_normalizes_root_flag_when_false() {
        let mut a = root_flag_false();
        assert!(!a.root.is_root, "fixture must start with is_root: false");
        let fixes = auto_fix(&mut a);
        assert_eq!(fixes.len(), 1);
        assert_eq!(fixes[0].kind, "root_flag_normalize");
        assert!(
            fixes[0].summary.contains("root"),
            "summary should name the offending node id; got {:?}",
            fixes[0].summary
        );
        assert!(a.root.is_root, "after auto_fix, root.is_root must be true");
    }

    #[test]
    fn auto_fix_then_validate_pass_when_only_root_flag_was_wrong() {
        // Mirrors the 2026-07-17 scene.beui scenario: editor saves
        // an asset whose root had `is_root: false`; auto_fix sets it
        // to true; the subsequent validate() must pass.
        let mut a = root_flag_false();
        auto_fix(&mut a);
        validate(&a).expect("post-fix asset must validate clean");
    }
}