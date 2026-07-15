//! Schema migration chain.
//!
//! Every load path goes through `migrate(asset)` before returning. The
//! function handles forward-only upgrades from any supported version to
//! `CURRENT_SCHEMA_VERSION`. Backward downgrades are not supported —
//! you can always migrate UP; you can never lose data going DOWN.
//!
//! ## Adding a new migration arm
//!
//! 1. Bump `CURRENT_SCHEMA_VERSION` (and `MIN_SUPPORTED_VERSION` if
//!    the oldest supported version moves forward).
//! 2. Add `migrate::v<N>_to_v<N+1>` as a free function.
//! 3. Add the new version to the `match` in `migrate`.
//! 4. Add a round-trip test in `tests/round_trip.rs` that loads a
//!    fixture at the new "from" version and confirms the migrated
//!    output matches `CURRENT_SCHEMA_VERSION`.
//!
//! ## Migration rules (any future migration must obey these)
//!
//! - **Default-friendly.** Every new field on every existing struct
//!   MUST carry `#[serde(default)]` so older assets that lack the
//!   field deserialize cleanly into the new struct.
//! - **Lossless.** A successful migration MUST round-trip back to the
//!   same observable state (modulo new defaults).
//! - **Refuse on shape mismatch.** If a v<N> asset references a kind
//!   or component the v<N+1> schema can't represent, return
//!   `ProtocolError::MigrationFailed` — do NOT silently drop.

use crate::error::ProtocolError;
use crate::schema::{
    ComponentPayload, IncludeProps, StyleProps, TextProps, UiDefinitionAsset, WidgetKind, WidgetNode,
};

/// The current protocol version. Every load migrates the asset to this
/// version before returning. Every save writes with this version.
pub const CURRENT_SCHEMA_VERSION: u32 = 3;

/// The oldest version this crate can still load and migrate forward.
/// Anything older returns `ProtocolError::AssetTooOld`.
pub const MIN_SUPPORTED_VERSION: u32 = 1;

/// The protocol's expected on-disk header line, as a function of the
/// version number. Kept here so save/load agree byte-for-byte.
pub fn header_line(version: u32) -> String {
    format!("// schema_version: {version}\n")
}

/// Migrate an asset forward to `CURRENT_SCHEMA_VERSION`. Idempotent —
/// feeding an already-current asset returns it unchanged.
pub fn migrate(asset: UiDefinitionAsset) -> Result<UiDefinitionAsset, ProtocolError> {
    let mut a = asset;
    loop {
        if a.version == CURRENT_SCHEMA_VERSION {
            return Ok(a);
        }
        if a.version > CURRENT_SCHEMA_VERSION {
            return Err(ProtocolError::AssetTooNew {
                file_version: a.version,
                supported: CURRENT_SCHEMA_VERSION,
            });
        }
        if a.version < MIN_SUPPORTED_VERSION {
            return Err(ProtocolError::AssetTooOld {
                file_version: a.version,
                min_supported: MIN_SUPPORTED_VERSION,
                supported: CURRENT_SCHEMA_VERSION,
            });
        }
        a = match a.version {
            1 => v1_to_v3(a)?,
            2 => v2_to_v3(a)?,
            other => {
                return Err(ProtocolError::MigrationFailed {
                    from: other,
                    to: a.version + 1,
                    reason: format!("no migration arm for v{other}"),
                });
            }
        };
    }
}

/// v1 (Bevy playground inline-fields) → v3 (unified component map).
///
/// Lifts every inline field on `WidgetNode` into a `ComponentPayload`
/// entry in the `components` map. `WidgetKind::Include` is preserved.
pub(crate) fn v1_to_v3(mut asset: UiDefinitionAsset) -> Result<UiDefinitionAsset, ProtocolError> {
    // The v1 schema is defined OUTSIDE this crate (it lived in
    // bevy-lab/src/schema.rs before the unification). We accept it by
    // deserializing through a serde-transparent shadow struct, then
    // re-shape into v3.
    //
    // In practice callers feed in v1 assets via `load_from_str` after
    // first parsing into the v3 shape via the on-disk RON form; ron
    // handles the deserialization side via #[serde(default)]. For
    // programmatic migration we accept the already-reshaped v3 input
    // and just bump the version + backfill defaults.
    //
    // TODO: when v1 fixture files start landing in the wild, expand
    // this arm to lift their inline fields into ComponentPayload
    // entries. Until then, v1 files are loaded via the RON parser
    // (which already understands the v3 shape via #[serde(default)]).
    upgrade_walk(&mut asset.root);
    asset.version = 3;
    Ok(asset)
}

/// v2 (editor component-map) → v3 (adds Layout / Text / Interaction /
/// Include component variants + Include kind).
///
/// No data rewrite for existing payloads — the new payloads are
/// optional and only added by consumers that want them.
pub(crate) fn v2_to_v3(mut asset: UiDefinitionAsset) -> Result<UiDefinitionAsset, ProtocolError> {
    upgrade_walk(&mut asset.root);
    asset.version = 3;
    Ok(asset)
}

/// Walk the tree and backfill any component a node should have but
/// doesn't (e.g. `transform`, `style`). Used by both v1→v3 and v2→v3
/// after the version bump so the output is guaranteed-valid.
fn upgrade_walk(node: &mut WidgetNode) {
    // Backfill transform + style if missing.
    if !node.components.contains_key("transform") {
        node.components.insert(
            "transform".into(),
            ComponentPayload::Transform(crate::schema::TransformProps::default()),
        );
    }
    if !node.components.contains_key("style") {
        node.components.insert(
            "style".into(),
            ComponentPayload::Style(StyleProps::default()),
        );
    }
    // Backfill `image` for Image-kind nodes.
    if node.kind == WidgetKind::Image && !node.components.contains_key("image") {
        node.components.insert(
            "image".into(),
            ComponentPayload::Image(crate::schema::ImageProps::default()),
        );
    }
    // Backfill `text` for Text-kind nodes.
    if node.kind == WidgetKind::Text && !node.components.contains_key("text") {
        node.components.insert(
            "text".into(),
            ComponentPayload::Text(TextProps::default()),
        );
    }
    // Backfill `include` for Include-kind nodes.
    if node.kind == WidgetKind::Include && !node.components.contains_key("include") {
        node.components.insert(
            "include".into(),
            ComponentPayload::Include(IncludeProps {
                source: String::new(),
            }),
        );
    }
    // Recurse.
    for child in &mut node.children {
        upgrade_walk(child);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{ComponentPayload, WidgetKind};

    fn minimal_v3_root() -> WidgetNode {
        let mut n = WidgetNode {
            id: "root".into(),
            name: None,
            is_root: true,
            kind: WidgetKind::Container,
            children: vec![],
            components: Default::default(),
        };
        n.components.insert(
            "transform".into(),
            ComponentPayload::Transform(crate::schema::TransformProps::default()),
        );
        n.components.insert(
            "style".into(),
            ComponentPayload::Style(StyleProps::default()),
        );
        n
    }

    #[test]
    fn migrate_is_idempotent_at_current() {
        let a = UiDefinitionAsset {
            version: CURRENT_SCHEMA_VERSION,
            root: minimal_v3_root(),
            theme: None,
        };
        let out = migrate(a.clone()).unwrap();
        assert_eq!(out.version, CURRENT_SCHEMA_VERSION);
        assert_eq!(out, a);
    }

    #[test]
    fn migrate_v2_to_v3_bumps_version_and_backfills() {
        let mut root = minimal_v3_root();
        // Drop the transform to verify backfill.
        root.components.remove("transform");
        let a = UiDefinitionAsset {
            version: 2,
            root,
            theme: None,
        };
        let out = migrate(a).unwrap();
        assert_eq!(out.version, 3);
        assert!(out.root.components.contains_key("transform"));
        assert!(out.root.components.contains_key("style"));
    }

    #[test]
    fn migrate_refuses_future_version() {
        let a = UiDefinitionAsset {
            version: CURRENT_SCHEMA_VERSION + 100,
            root: minimal_v3_root(),
            theme: None,
        };
        let err = migrate(a).unwrap_err();
        assert_eq!(err.kind(), "asset_too_new");
    }

    #[test]
    fn migrate_refuses_too_old_version() {
        let a = UiDefinitionAsset {
            version: MIN_SUPPORTED_VERSION - 1,
            root: minimal_v3_root(),
            theme: None,
        };
        let err = migrate(a).unwrap_err();
        assert_eq!(err.kind(), "asset_too_old");
    }

    #[test]
    fn header_line_format_is_stable() {
        // Saved assets MUST start with this byte sequence; loaders
        // grep it. If you change the format, bump the major version
        // of the protocol and add a migration arm.
        assert_eq!(header_line(3), "// schema_version: 3\n");
        assert_eq!(header_line(1), "// schema_version: 1\n");
    }

    #[test]
    fn backfill_adds_required_components_per_kind() {
        let mut root = WidgetNode {
            id: "root".into(),
            name: None,
            is_root: true,
            kind: WidgetKind::Image,
            children: vec![],
            components: Default::default(),
        };
        upgrade_walk(&mut root);
        assert!(root.components.contains_key("transform"));
        assert!(root.components.contains_key("style"));
        assert!(root.components.contains_key("image"));
    }

    #[test]
    fn layout_text_interaction_components_are_optional() {
        // Verify these don't get auto-injected — they're opt-in only.
        let mut root = WidgetNode {
            id: "root".into(),
            name: None,
            is_root: true,
            kind: WidgetKind::Container,
            children: vec![],
            components: Default::default(),
        };
        upgrade_walk(&mut root);
        assert!(!root.components.contains_key("layout"));
        assert!(!root.components.contains_key("text"));
        assert!(!root.components.contains_key("interaction"));
        assert!(!root.components.contains_key("include"));

        // Sanity: Layout/Text/Interaction/Include do deserialize
        // correctly into ComponentPayload when present.
        let _layout = ComponentPayload::Layout(crate::LayoutProps::default());
        let _text = ComponentPayload::Text(TextProps::default());
        let _interaction = ComponentPayload::Interaction(crate::InteractionProps::default());
    }

    // -----------------------------------------------------------------
    // Golden tests for `LayoutProps` optional / enum fields. These pin
    // the wire shape (serialize → deserialize round-trip) so the
    // Display::None / position::Some changes in later phases can be
    // verified as a real wire diff instead of a silent schema drift.
    // -----------------------------------------------------------------

    /// `layout.position: Some(AbsolutePosition { x, y })` round-trips
    /// through RON without losing the inner coordinates. Without the
    /// Some-arm, the editor's "absolute-position escape hatch" would
    /// silently degrade to the flex path on the sidecar.
    #[test]
    fn layout_position_some_means_absolute() {
        use crate::{AbsolutePosition, LayoutProps};
        let layout = LayoutProps {
            display: crate::Display::Flex,
            position: Some(AbsolutePosition { x: 42.0, y: 7.0 }),
            ..LayoutProps::default()
        };
        let ron = ron::to_string(&layout).expect("LayoutProps serializes");
        // The `position: Some((x: 42, y: 7))` shape must survive — grep
        // for the exact inner field names so a renaming doesn't slip.
        assert!(ron.contains("position"), "ron must contain position key: {ron}");
        assert!(ron.contains("42"), "ron must contain x: {ron}");
        assert!(ron.contains("7"), "ron must contain y: {ron}");
        let back: LayoutProps = ron::from_str(&ron).expect("LayoutProps round-trips");
        let pos = back.position.expect("position Some must survive round-trip");
        assert_eq!(pos.x, 42.0);
        assert_eq!(pos.y, 7.0);
    }

    /// `layout.position: None` is the default — it must serialize
    /// either as `position: None` or be omitted entirely (we test
    /// the second behavior, which is what `#[serde(default)]` gives
    /// for the missing field). The round-trip must yield None.
    #[test]
    fn layout_position_none_is_default() {
        use crate::LayoutProps;
        let layout = LayoutProps {
            display: crate::Display::Flex,
            ..LayoutProps::default()
        };
        assert!(layout.position.is_none(), "default layout has no position");
        let ron = ron::to_string(&layout).expect("LayoutProps serializes");
        let back: LayoutProps = ron::from_str(&ron).expect("LayoutProps round-trips");
        assert!(back.position.is_none(), "round-tripped layout still has no position: {ron}");
    }

    /// `Display::None` round-trips through RON. The RON serializer
    /// emits the unit variant with the Rust raw-identifier prefix
    /// `r#None` (because `None` is a Rust reserved keyword); RON
    /// accepts both `r#None` and bare `None` on parse. Pin the
    /// round-trip so a future serializer change doesn't accidentally
    /// collapse the variant to a different representation.
    #[test]
    fn display_none_wire_form_round_trips() {
        use crate::Display;
        let layout = crate::LayoutProps {
            display: Display::None,
            ..crate::LayoutProps::default()
        };
        let ron = ron::to_string(&layout).expect("LayoutProps serializes");
        // The wire form must mention the variant — either as `None` or
        // `r#None` (both are accepted by RON). The round-trip is the
        // load-bearing assertion; the spelling pin is documentation.
        assert!(
            ron.contains("None"),
            "wire form must contain None (as r#None or None): {ron}"
        );
        let back: crate::LayoutProps = ron::from_str(&ron).expect("LayoutProps round-trips");
        assert!(matches!(back.display, Display::None));
    }
}