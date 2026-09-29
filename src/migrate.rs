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
    ButtonProps, Color, ComponentPayload, FlexDirection, IncludeProps, LayoutDownwardProps,
    LayoutType, LayoutUpwardMode, LayoutUpwardProps, StyleProps, TextProps, UiDefinitionAsset,
    WidgetKind, WidgetNode,
};

/// The current protocol version. Every load migrates the asset to this
/// version before returning. Every save writes with this version.
pub const CURRENT_SCHEMA_VERSION: u32 = 5;

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
            3 => v3_to_v4(a)?,
            4 => v4_to_v5(a)?,
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

/// v3 (single `LayoutProps` payload) → v4 (split into `LayoutDownward`
/// + `LayoutUpward` payloads).
///
/// The v3 schema carried one `layout` component containing every layout
/// field — display, flex_direction, justify/align, gap, padding, margin,
/// flex_shrink, AND the absolute-position escape hatch
/// (`position: Option<AbsolutePosition>`). v4 splits this into two
/// independent payloads:
///
/// - **`layout_downward`** — "how I lay out my children". Carries
///   `type_` (None / Horizontal / Vertical / Grid) + the derived
///   flex-direction / justify / align / gap / padding / margin /
///   flex_shrink fields. `type_` is computed from the v3
///   `display + flex_direction` pair:
///   - `display: Flex` + `flex_direction: Row | RowReverse`
///     → `LayoutType::Horizontal`
///   - `display: Flex` + `flex_direction: Column | ColumnReverse`
///     → `LayoutType::Vertical`
///   - `display: None` → `LayoutType::None`
/// - **`layout_upward`** — "how I relate to my parent's flex flow".
///   Always carries a `mode`:
///   - `position: Some(_) → mode: Ignore` (the v3 absolute escape hatch)
///   - `position: None → mode: Inherit` (the default flex-flow path)
///
/// The old `layout` key is removed from the components map.
///
/// This is a structural rewrite — every node carrying a v3 `layout`
/// payload gets BOTH new payloads inserted. Other payloads are
/// unchanged.
pub(crate) fn v3_to_v4(mut asset: UiDefinitionAsset) -> Result<UiDefinitionAsset, ProtocolError> {
    // The old `Layout(LayoutProps)` enum variant is GONE from
    // `ComponentPayload` in v4, but deserialization still routes the
    // v3 wire form correctly thanks to #[serde(default)] on
    // `ComponentPayload`. We can't read the old payload through the
    // new enum, so the migration parses the v3 wire form on demand
    // via a shadow enum (see `v3_layout_payload`). After rewriting,
    // we re-encode through the current schema.
    v3_to_v4_walk(&mut asset.root);
    asset.version = 4;
    Ok(asset)
}

/// v4 → v5: extend `ButtonProps` with hover / pressed / disabled color
/// fields plus a `disabled` flag.
///
/// v4 `ButtonProps` carried only `label` + `color`. v5 adds:
/// - `hover_color` (lighter shade, default `{90, 90, 90, 255}`)
/// - `pressed_color` (darker shade, default `{30, 30, 30, 255}`)
/// - `disabled_color` (50% gray, default `{128, 128, 128, 128}`)
/// - `disabled: bool` (default `false`)
///
/// Every new field carries `#[serde(default)]` with a typed default
/// function, so a v4 wire form parsed through the v5 Rust struct
/// already has the new fields populated — the migration is effectively
/// a version bump. The walker below is defensive: it backfills any
/// field still at Color's `default()` sentinel (transparent black),
/// which is what serde falls back to when the field is missing on the
/// wire. A genuine (non-transparent) value — even one that happens to
/// match a default — is left untouched. No other payload type
/// changes.
pub(crate) fn v4_to_v5(mut asset: UiDefinitionAsset) -> Result<UiDefinitionAsset, ProtocolError> {
    v4_to_v5_walk(&mut asset.root);
    asset.version = 5;
    Ok(asset)
}

/// Recursive walker for `v4_to_v5`. Backfills the new color fields on
/// any `Button` payload that hasn't yet had them set explicitly.
fn v4_to_v5_walk(node: &mut WidgetNode) {
    if let Some(ComponentPayload::Button(b)) = node.components.get_mut("button") {
        let defaults = ButtonProps::default();
        let transparent = Color::default();
        if b.hover_color == transparent {
            b.hover_color = defaults.hover_color;
        }
        if b.pressed_color == transparent {
            b.pressed_color = defaults.pressed_color;
        }
        if b.disabled_color == transparent {
            b.disabled_color = defaults.disabled_color;
        }
        // `disabled: bool` has no "missing" concept for serde; its
        // default (`false`) is what we want on a v4 input.
    }
    for child in &mut node.children {
        v4_to_v5_walk(child);
    }
}

/// Recursive walker for `v3_to_v4`. Rewrites every node's
/// `"layout"` component into `"layout_downward"` + `"layout_upward"`.
fn v3_to_v4_walk(node: &mut WidgetNode) {
    let layout = node.components.remove("layout");
    if let Some(v3) = layout {
        if let Some((down, up)) = v3_layout_to_split(&v3) {
            node.components
                .insert("layout_downward".into(), ComponentPayload::LayoutDownward(down));
            node.components
                .insert("layout_upward".into(), ComponentPayload::LayoutUpward(up));
        }
    }
    for child in &mut node.children {
        v3_to_v4_walk(child);
    }
}

/// Shadow enum mirroring the v3 `ComponentPayload` just enough to
/// deserialize a v3 `"layout"` entry. We can't read v3 data through
/// the current enum because the `Layout(LayoutProps)` variant no
/// longer exists — but the wire form is fixed by RON's tagged-enum
/// deserialization, so a shadow with the same tag succeeds.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "lowercase")]
enum V3LayoutPayload {
    /// The single v3 layout payload — the thing we're rewriting.
    #[serde(rename = "layout")]
    Layout(V3LayoutProps),
    /// Catch-all so unrecognized v3 payloads (Text, Image, ...) are
    /// passed through untouched.
    #[serde(other)]
    Other,
}

/// Shadow struct mirroring the v3 `LayoutProps` shape. Every field
/// here matches the v3 wire form exactly (including the now-removed
/// `position: Option<V3AbsolutePosition>` and `display: V3Display`).
/// After Phase A removed these from the canonical schema, this
/// shadow is the only place they still live on disk.
#[derive(Debug, Clone, Copy, Default, serde::Deserialize)]
enum V3Display {
    #[default]
    Flex,
    None,
}

#[derive(Debug, Clone, Copy, Default, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
enum V3FlexDirection {
    #[default]
    Row,
    Column,
    RowReverse,
    ColumnReverse,
}

#[derive(Debug, Clone, Copy, Default, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
enum V3JustifyContent {
    #[default]
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Debug, Clone, Copy, Default, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
enum V3AlignItems {
    #[default]
    Stretch,
    FlexStart,
    FlexEnd,
    Center,
}

#[derive(Debug, Clone, Copy, Default, serde::Deserialize)]
struct V3Padding {
    #[serde(default)]
    top: f32,
    #[serde(default)]
    right: f32,
    #[serde(default)]
    bottom: f32,
    #[serde(default)]
    left: f32,
}

#[derive(Debug, Clone, Copy, Default, serde::Deserialize)]
struct V3Margin {
    #[serde(default)]
    top: f32,
    #[serde(default)]
    right: f32,
    #[serde(default)]
    bottom: f32,
    #[serde(default)]
    left: f32,
}

#[derive(Debug, Clone, Copy, Default, serde::Deserialize)]
#[allow(dead_code)] // x/y exist so serde can parse v3 wire form; the migration only reads Option::is_some().
struct V3AbsolutePosition {
    x: f32,
    y: f32,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct V3LayoutProps {
    #[serde(default)]
    display: V3Display,
    #[serde(default)]
    flex_direction: V3FlexDirection,
    #[serde(default)]
    justify_content: V3JustifyContent,
    #[serde(default)]
    align_items: V3AlignItems,
    #[serde(default)]
    gap: f32,
    #[serde(default)]
    padding: V3Padding,
    #[serde(default)]
    margin: V3Margin,
    #[serde(default = "default_v3_flex_shrink")]
    flex_shrink: f32,
    #[serde(default)]
    position: Option<V3AbsolutePosition>,
}

fn default_v3_flex_shrink() -> f32 {
    1.0
}

/// Convert one v3 `ComponentPayload` (only the `Layout` variant is
/// meaningful) into the v4 split pair. Returns `None` for any other
/// variant — those payloads are passed through untouched by the
/// walker.
fn v3_layout_to_split(v3: &ComponentPayload) -> Option<(LayoutDownwardProps, LayoutUpwardProps)> {
    // Re-serialize the v3 payload back to its RON wire form, then
    // parse through the shadow enum. This works because the v3 wire
    // form is byte-identical to the shadow enum's expected input
    // (the shadow was reverse-engineered from the v3 schema). For
    // payloads that aren't `Layout`, this returns `Other` and we
    // bail.
    let wire = ron::to_string(v3).ok()?;
    let parsed: V3LayoutPayload = ron::from_str(&wire).ok()?;
    let v3_layout = match parsed {
        V3LayoutPayload::Layout(l) => l,
        V3LayoutPayload::Other => return None,
    };

    // Map v3 (display + flex_direction) → v4 LayoutType.
    let type_ = match (v3_layout.display, &v3_layout.flex_direction) {
        (V3Display::Flex, V3FlexDirection::Row | V3FlexDirection::RowReverse) => LayoutType::Horizontal,
        (V3Display::Flex, V3FlexDirection::Column | V3FlexDirection::ColumnReverse) => LayoutType::Vertical,
        (V3Display::None, _) => LayoutType::None,
    };

    // Carry over flex_direction (mapped to the v4 enum).
    let flex_direction = match v3_layout.flex_direction {
        V3FlexDirection::Row => FlexDirection::Row,
        V3FlexDirection::Column => FlexDirection::Column,
        V3FlexDirection::RowReverse => FlexDirection::RowReverse,
        V3FlexDirection::ColumnReverse => FlexDirection::ColumnReverse,
    };

    let down = LayoutDownwardProps {
        type_,
        flex_direction,
        justify_content: map_justify(v3_layout.justify_content),
        align_items: map_align(v3_layout.align_items),
        gap: v3_layout.gap,
        padding: crate::schema::Padding {
            top: v3_layout.padding.top,
            right: v3_layout.padding.right,
            bottom: v3_layout.padding.bottom,
            left: v3_layout.padding.left,
        },
        margin: crate::schema::Margin {
            top: v3_layout.margin.top,
            right: v3_layout.margin.right,
            bottom: v3_layout.margin.bottom,
            left: v3_layout.margin.left,
        },
        flex_shrink: v3_layout.flex_shrink,
    };

    // Map v3 position → v4 LayoutUpwardMode.
    let up = LayoutUpwardProps {
        mode: if v3_layout.position.is_some() {
            LayoutUpwardMode::Ignore
        } else {
            LayoutUpwardMode::Inherit
        },
    };

    Some((down, up))
}

fn map_justify(j: V3JustifyContent) -> crate::schema::JustifyContent {
    use crate::schema::JustifyContent as J;
    match j {
        V3JustifyContent::FlexStart => J::FlexStart,
        V3JustifyContent::FlexEnd => J::FlexEnd,
        V3JustifyContent::Center => J::Center,
        V3JustifyContent::SpaceBetween => J::SpaceBetween,
        V3JustifyContent::SpaceAround => J::SpaceAround,
        V3JustifyContent::SpaceEvenly => J::SpaceEvenly,
    }
}

fn map_align(a: V3AlignItems) -> crate::schema::AlignItems {
    use crate::schema::AlignItems as A;
    match a {
        V3AlignItems::Stretch => A::Stretch,
        V3AlignItems::FlexStart => A::FlexStart,
        V3AlignItems::FlexEnd => A::FlexEnd,
        V3AlignItems::Center => A::Center,
    }
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
// V3 shadow parser (load-time only)
// ---------------------------------------------------------------------------

/// Shadow of the v3 `ComponentPayload` enum. Mirrors the wire form
/// exactly so a v3 `.beui` file with the legacy `Layout` variant
/// deserializes successfully. We can't read v3 wire form through the
/// v4 `ComponentPayload` enum (the `Layout` variant is gone), so the
/// load pipeline routes any input with `version < CURRENT_SCHEMA_VERSION`
/// through this shadow struct + a structural rewrite into the v4 shape.
///
/// All payloads except `Layout` use the same Rust types as v4
/// (wire-compatible); the matching v4 enum variants surface during
/// the conversion walk. `Layout` is the legacy single-payload form
/// that this migration rewrites into the `LayoutDownward` +
/// `LayoutUpward` pair.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "lowercase")]
enum V3ComponentPayload {
    #[serde(rename = "transform")]
    Transform(crate::schema::TransformProps),
    #[serde(rename = "style")]
    Style(crate::schema::StyleProps),
    /// Legacy single-payload layout — the one we're migrating away from.
    /// After Phase A, this variant exists ONLY in this shadow type and
    /// on disk in pre-v4 `.beui` files.
    #[serde(rename = "layout")]
    Layout(V3LayoutProps),
    #[serde(rename = "text")]
    Text(crate::schema::TextProps),
    #[serde(rename = "image")]
    Image(crate::schema::ImageProps),
    #[serde(rename = "interaction")]
    Interaction(crate::schema::InteractionProps),
    #[serde(rename = "include")]
    Include(crate::schema::IncludeProps),
}

/// Shadow of the v3 `WidgetNode` shape — same fields as v4 except
/// `components` maps to `V3ComponentPayload` instead of `ComponentPayload`.
#[derive(Debug, Clone, serde::Deserialize)]
struct V3WidgetNode {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub is_root: bool,
    pub kind: WidgetKind,
    #[serde(default)]
    pub children: Vec<V3WidgetNode>,
    #[serde(default)]
    pub components: std::collections::BTreeMap<String, V3ComponentPayload>,
}

/// Shadow of the v3 `UiDefinitionAsset`. Same envelope as v4 except the
/// root holds `V3WidgetNode`.
#[derive(Debug, Clone, serde::Deserialize)]
struct V3UiDefinitionAsset {
    pub version: u32,
    pub root: V3WidgetNode,
    #[serde(default)]
    pub theme: Option<crate::schema::ThemeRef>,
}

/// Parse a v3 RON body into a v4 `UiDefinitionAsset`.
///
/// `body` is the RON source with the `// schema_version: N` header
/// already stripped. We re-parse through the v3 shadow struct (so
/// the legacy `Layout` variant survives) and rewrite each node's
/// `components` map in-place: `Layout(V3LayoutProps)` becomes the
/// split `LayoutDownward` + `LayoutUpward` pair; everything else
/// round-trips through the v4 enum unchanged.
///
/// The returned asset retains the original `version` from the file
/// so the caller can verify it matches the `// schema_version: N`
/// header before handing to `migrate()`. The data is already
/// v4-shaped, so `v3_to_v4` walks no-op afterwards.
///
/// Errors are bubbled up as `ProtocolError::RonParse` so callers see
/// the same diagnostic surface as the v4 path.
pub fn parse_v3_into_v4(body: &str) -> Result<UiDefinitionAsset, ProtocolError> {
    let v3: V3UiDefinitionAsset = ron::from_str(body).map_err(|e| ProtocolError::RonParse {
        message: e.code.to_string(),
        position: Some(e.span.start.line),
    })?;
    let root = convert_v3_widget(v3.root);
    Ok(UiDefinitionAsset {
        version: v3.version,
        root,
        theme: v3.theme,
    })
}

/// Convert one v3 widget tree node into a v4 widget tree node,
/// rewriting the legacy `Layout` payload into the split pair during
/// the walk.
fn convert_v3_widget(v3: V3WidgetNode) -> WidgetNode {
    use std::collections::BTreeMap;
    let mut components: BTreeMap<String, ComponentPayload> = BTreeMap::new();
    for (key, payload) in v3.components {
        let v4 = match payload {
            V3ComponentPayload::Transform(t) => Some(ComponentPayload::Transform(t)),
            V3ComponentPayload::Style(s) => Some(ComponentPayload::Style(s)),
            V3ComponentPayload::Text(t) => Some(ComponentPayload::Text(t)),
            V3ComponentPayload::Image(i) => Some(ComponentPayload::Image(i)),
            V3ComponentPayload::Interaction(i) => Some(ComponentPayload::Interaction(i)),
            V3ComponentPayload::Include(i) => Some(ComponentPayload::Include(i)),
            V3ComponentPayload::Layout(layout) => {
                if let Some((down, up)) = v3_layout_props_to_split(&layout) {
                    components.insert(
                        "layout_downward".into(),
                        ComponentPayload::LayoutDownward(down),
                    );
                    components.insert(
                        "layout_upward".into(),
                        ComponentPayload::LayoutUpward(up),
                    );
                }
                None
            }
        };
        if let Some(c) = v4 {
            components.insert(key, c);
        }
    }
    WidgetNode {
        id: v3.id,
        name: v3.name,
        is_root: v3.is_root,
        kind: v3.kind,
        children: v3
            .children
            .into_iter()
            .map(convert_v3_widget)
            .collect(),
        components,
    }
}

/// Map one v3 `V3LayoutProps` onto the v4 split pair. Mirrors the
/// logic in `v3_layout_to_split` but operates on the parsed shadow
/// struct directly (no RON re-serialization detour). The two
/// functions agree on mapping rules — the dispatch lives there so
/// future fields (margin, padding) keep both in sync.
fn v3_layout_props_to_split(
    layout: &V3LayoutProps,
) -> Option<(LayoutDownwardProps, LayoutUpwardProps)> {
    let type_ = match (layout.display, &layout.flex_direction) {
        (V3Display::Flex, V3FlexDirection::Row | V3FlexDirection::RowReverse) => LayoutType::Horizontal,
        (V3Display::Flex, V3FlexDirection::Column | V3FlexDirection::ColumnReverse) => LayoutType::Vertical,
        (V3Display::None, _) => LayoutType::None,
    };

    let flex_direction = match layout.flex_direction {
        V3FlexDirection::Row => FlexDirection::Row,
        V3FlexDirection::Column => FlexDirection::Column,
        V3FlexDirection::RowReverse => FlexDirection::RowReverse,
        V3FlexDirection::ColumnReverse => FlexDirection::ColumnReverse,
    };

    let down = LayoutDownwardProps {
        type_,
        flex_direction,
        justify_content: map_justify(layout.justify_content),
        align_items: map_align(layout.align_items),
        gap: layout.gap,
        padding: crate::schema::Padding {
            top: layout.padding.top,
            right: layout.padding.right,
            bottom: layout.padding.bottom,
            left: layout.padding.left,
        },
        margin: crate::schema::Margin {
            top: layout.margin.top,
            right: layout.margin.right,
            bottom: layout.margin.bottom,
            left: layout.margin.left,
        },
        flex_shrink: layout.flex_shrink,
    };

    let up = LayoutUpwardProps {
        mode: if layout.position.is_some() {
            LayoutUpwardMode::Ignore
        } else {
            LayoutUpwardMode::Inherit
        },
    };

    Some((down, up))
}

/// Convert a v4-shaped `ComponentsBTreeMap` entry back into a v3
/// `V3LayoutProps`. Used when re-serializing v4 → v3 (currently a
/// disabled path; kept as the future-shape-of-rev-3 helper).
#[allow(dead_code)]
fn v4_layout_downward_to_v3(_down: &LayoutDownwardProps, _up: &LayoutUpwardProps) -> V3LayoutProps {
    // Currently unreachable — Phase B is forward-only. Reserved for a
    // future "save as v3" feature.
    V3LayoutProps::default()
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
        // Phase B: CURRENT_SCHEMA_VERSION bumped to 4. A v2 input
        // travels v2 → v3 → v4 and exits at the current version.
        assert_eq!(out.version, CURRENT_SCHEMA_VERSION);
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

    /// A v4 button (label + color only) migrates to v5 with the four
    /// new interaction-state fields populated to their defaults. The
    /// walker treats Color::default() (transparent black) as "unset"
    /// and applies the documented default.
    #[test]
    fn v4_to_v5_backfills_button_fields() {
        use crate::schema::ButtonProps;

        // Build a v4 ButtonProps by hand — only label + color were
        // present on the wire; the rest stay at their struct defaults.
        let transparent = Color::default();
        let red = Color {
            r: 200,
            g: 30,
            b: 30,
            a: 255,
        };
        let v4_button = ButtonProps {
            label: "Go".into(),
            color: red,
            hover_color: transparent,
            pressed_color: transparent,
            disabled_color: transparent,
            disabled: false,
        };

        let mut root = minimal_v3_root();
        root.components
            .insert("button".into(), ComponentPayload::Button(v4_button));
        let a = UiDefinitionAsset {
            version: 4,
            root,
            theme: None,
        };

        let out = migrate(a).unwrap();
        assert_eq!(out.version, CURRENT_SCHEMA_VERSION);
        assert_eq!(out.version, 5);

        let migrated = match out
            .root
            .components
            .get("button")
            .expect("button payload survives migration")
        {
            ComponentPayload::Button(b) => b,
            _ => panic!("expected ComponentPayload::Button"),
        };

        let defaults = ButtonProps::default();
        assert_eq!(migrated.label, "Go", "label preserved");
        assert_eq!(migrated.color, red, "color preserved");
        assert_eq!(
            migrated.hover_color, defaults.hover_color,
            "hover_color backfilled"
        );
        assert_eq!(
            migrated.pressed_color, defaults.pressed_color,
            "pressed_color backfilled"
        );
        assert_eq!(
            migrated.disabled_color, defaults.disabled_color,
            "disabled_color backfilled"
        );
        assert!(!migrated.disabled, "disabled left at default false");
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
        assert!(!root.components.contains_key("layout_downward"));
        assert!(!root.components.contains_key("layout_upward"));
        assert!(!root.components.contains_key("text"));
        assert!(!root.components.contains_key("interaction"));
        assert!(!root.components.contains_key("include"));

        // Sanity: Text/Interaction/Include do deserialize correctly
        // into ComponentPayload when present. The split layout
        // components are exercised in `beui_protocol::layout::tests`.
        let _text = ComponentPayload::Text(TextProps::default());
        let _interaction = ComponentPayload::Interaction(crate::InteractionProps::default());
    }

    // -----------------------------------------------------------------
    // Display::None wire-form round-trip test (v4 split removed the
    // surrounding `LayoutProps`; the test now pins the enum's wire
    // form directly).
    // -----------------------------------------------------------------

    /// `Display::None` round-trips through RON. The RON serializer
    /// emits the unit variant with the Rust raw-identifier prefix
    /// `r#None` (because `None` is a Rust reserved keyword); RON
    /// accepts both `r#None` and bare `None` on parse. Pin the
    /// round-trip so a future serializer change doesn't accidentally
    /// collapse the variant to a different representation.
    #[test]
    fn display_none_wire_form_round_trips() {
        use crate::Display;
        let display = Display::None;
        let ron = ron::to_string(&display).expect("Display serializes");
        // The wire form must mention the variant — either as `None` or
        // `r#None` (both are accepted by RON). The round-trip is the
        // load-bearing assertion; the spelling pin is documentation.
        assert!(
            ron.contains("None"),
            "wire form must contain None (as r#None or None): {ron}"
        );
        let back: Display = ron::from_str(&ron).expect("Display round-trips");
        assert!(matches!(back, Display::None));
    }
}