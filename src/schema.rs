//! Canonical `.beui` schema (protocol v3).
//!
//! This is the **single source of truth** for the `.beui` file format.
//! Every producer and every consumer — the editor, the sidecar renderer,
//! and any external Bevy project that wants to author or load UI — uses
//! these types and nothing else.
//!
//! ## v3 design
//!
//! v3 unifies two previously-divergent schemas (the editor's v2 with
//! `components: { ... }` map, and the Bevy playground's v1 with inline
//! `layout:`/`style:`/`text:`/`image:`/`include:` fields):
//!
//! - The **outer shape is the component map** (v2's structure) — a
//!   `BTreeMap<String, ComponentPayload>` keyed by component type, so
//!   adding a new component does not change the outer schema.
//! - v1's inline fields are **promoted to component payloads**:
//!   `LayoutProps`, `TextProps`, `InteractionProps`, `IncludeProps`,
//!   `ImageProps` (already a payload in v2), `StyleProps` (already).
//! - **`Include` is a new `WidgetKind` variant** — a node with
//!   `kind: Include` MUST carry an `include` component.
//! - **Colors stay sRGB `u8` 0-255** (the editor's convention). Bevy
//!   converts via `Color::srgba(r/255, g/255, b/255, a/255)` at render
//!   time.
//!
//! ## Migration
//!
//! - v1 → v3: lift every inline field into a `ComponentPayload` entry
//!   (`layout:` → `Layout`, `style:` → `Style`, `text:` → `Text`,
//!   `image:` → `Image`, `include:` → `Include`). Done in
//!   `migrate::v1_to_v3`.
//! - v2 → v3: add `Layout`, `Text`, `Interaction`, `Include` payload
//!   variants to the component map (no data rewrite for existing
//!   payloads). Done in `migrate::v2_to_v3`.
//!
//! ## Invariants
//!
//! Enforced by `validate::validate`. A conforming `.beui` asset MUST:
//! 1. Have exactly one widget with `is_root: true`.
//! 2. Have all widget ids unique within the tree.
//! 3. Have every widget carry `transform` and `style` components.
//! 4. Have every `Image` widget carry an `image` component.
//! 5. Have every `Include` widget carry an `include` component.
//! 6. Have every `Text` widget carry a `text` component.
//! 7. Have `version == CURRENT_SCHEMA_VERSION` after migration.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Asset envelope
// ---------------------------------------------------------------------------

/// Top-level `.beui` asset.
///
/// The on-disk form is RON. The first line is always a comment of the
/// form `// schema_version: N` where N equals `asset.version`. The
/// `io::save_to_string` writer emits this header; `io::load_from_str`
/// verifies it matches the body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiDefinitionAsset {
    /// Schema version of this asset. On load, the protocol crate
    /// migrates this to `CURRENT_SCHEMA_VERSION` before returning.
    pub version: u32,
    /// The single root widget. The tree is owned (deep copy on move).
    pub root: WidgetNode,
    /// Optional reference to a shared theme file (colors, typography).
    /// Editor-side tooling uses this to swap visual presets.
    #[serde(default)]
    pub theme: Option<ThemeRef>,
}

/// Reference to a shared theme. Resolved by the consumer relative to
/// the asset's containing folder; the protocol does not interpret paths.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemeRef {
    /// Theme name (e.g. `"default-dark"`).
    pub name: String,
    /// Optional path to the theme's RON file.
    #[serde(default)]
    pub path: Option<String>,
}

// ---------------------------------------------------------------------------
// Widget Node
// ---------------------------------------------------------------------------

/// A single widget in the tree. Children are owned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetNode {
    /// Stable string id. MUST be unique within the tree
    /// (enforced by `validate`).
    pub id: String,
    /// Optional human-readable label. Editor-only metadata — the
    /// `io` module's save path MAY strip this. Consumers MUST NOT
    /// rely on it for rendering.
    #[serde(default)]
    pub name: Option<String>,
    /// At most one node per asset may carry `is_root: true`
    /// (enforced by `validate`).
    #[serde(default)]
    pub is_root: bool,
    /// What kind of widget this is. See [`WidgetKind`].
    pub kind: WidgetKind,
    /// Owned children. Empty list is valid.
    #[serde(default)]
    pub children: Vec<WidgetNode>,
    /// Component payloads, keyed by component type name
    /// (e.g. `"transform"`, `"style"`, `"image"`).
    ///
    /// `BTreeMap` (not `HashMap`) so RON output is sorted, stable, and
    /// round-trips byte-identically.
    #[serde(default)]
    pub components: BTreeMap<String, ComponentPayload>,
}

/// Widget kinds. The set is **closed** — adding a new kind is a
/// breaking schema change. To add behavior, add a new component payload
/// to the existing kind instead.
///
/// `Include` is special: it must carry an `include` component and its
/// children are ignored (the referenced file's tree is inlined at load
/// time by the consumer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum WidgetKind {
    /// A generic container with no rendering of its own. Used as a
    /// layout/div node; carries `transform` + `style` and (optionally)
    /// `layout` to lay out children.
    Container,
    /// A text-rendering widget. MUST carry a `text` component
    /// (enforced by `validate`).
    Text,
    /// An image-rendering widget. MUST carry an `image` component
    /// (enforced by `validate`).
    Image,
    /// A clickable button. Carries a `text` component for its label
    /// and an `interaction` component for the click handler.
    Button,
    /// A text-input field. Renders as a text field with a caret.
    TextInput,
    /// A checkable toggle. Carries an `interaction` component.
    Checkbox,
    /// A scrollable container. Children are clipped to the visible
    /// region; the consumer handles scroll input.
    ScrollView,
    /// A progress bar. Renders a fill bar over a track.
    ProgressBar,
    /// Reference to another `.beui` file. Carries an `include`
    /// component; children are ignored at load time.
    Include,
}

impl Default for WidgetKind {
    fn default() -> Self {
        WidgetKind::Container
    }
}

// ---------------------------------------------------------------------------
// Bevy `Val`
// ---------------------------------------------------------------------------

/// Mirrors Bevy's `Val` enum. A discriminated length value: pixels
/// (canvas-space), percent of the parent's content box, or `Auto`.
///
/// On-disk spelling is `PascalCase` (`Px`, `Percent`, `Auto`) — the
/// form the original editor fixtures ship in. If you change this,
/// either rewrite every existing fixture or add a migration arm.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Val {
    /// Length in canvas-space pixels. Absolute regardless of parent size.
    Px(f32),
    /// Length as a percentage of the parent's content box. `0.0` is
    /// none, `100.0` fills the box, `>100.0` overflows.
    Percent(f32),
    /// Length determined by the content (or the consumer's default).
    Auto,
}

impl Val {
    /// Construct a `Val::Px`.
    pub const fn px(v: f32) -> Self {
        Val::Px(v)
    }
    /// Construct a `Val::Percent`.
    pub const fn pct(v: f32) -> Self {
        Val::Percent(v)
    }
    /// Construct a `Val::Auto`.
    pub const fn auto() -> Self {
        Val::Auto
    }
}

impl Default for Val {
    fn default() -> Self {
        Val::Auto
    }
}

// ---------------------------------------------------------------------------
// Position / Size / Scale / Flip
// ---------------------------------------------------------------------------

/// X/Y position in the parent's coordinate space, using `Val` units.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct PositionVal {
    /// Horizontal position.
    pub x: Val,
    /// Vertical position.
    pub y: Val,
}

/// Width/height of the widget, using `Val` units.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SizeVal {
    /// Width.
    pub width: Val,
    /// Height.
    pub height: Val,
}

impl Default for SizeVal {
    fn default() -> Self {
        SizeVal {
            width: Val::Px(100.0),
            height: Val::Px(100.0),
        }
    }
}

/// Scale factors applied to the widget's rendered content. `1.0` is
/// identity; `2.0` doubles; `0.0` collapses the widget.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Scale {
    /// Horizontal scale factor.
    pub x: f32,
    /// Vertical scale factor.
    pub y: f32,
}

impl Default for Scale {
    fn default() -> Self {
        Scale { x: 1.0, y: 1.0 }
    }
}

/// Axis-mirror flags applied to the widget's content (image flip,
/// layout direction reversal). Editor-only metadata today.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Flip {
    /// Flip horizontally.
    pub x: bool,
    /// Flip vertically.
    pub y: bool,
}

// ---------------------------------------------------------------------------
// Color (sRGB u8 0-255)
// ---------------------------------------------------------------------------

/// sRGB color in 0-255 channels. Bevy converts at render time via
/// `Color::srgba(r/255, g/255, b/255, a/255)`. Alpha 0 = fully
/// transparent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    /// Red channel, 0-255.
    pub r: u8,
    /// Green channel, 0-255.
    pub g: u8,
    /// Blue channel, 0-255.
    pub b: u8,
    /// Alpha channel, 0=transparent, 255=opaque.
    pub a: u8,
}

impl Default for Color {
    fn default() -> Self {
        // Fully transparent black — invisible by default. Users opt in
        // with `fill_enabled: true` + non-zero alpha.
        Color {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        }
    }
}

impl Color {
    /// Opaque black. Useful as a default border color so a non-zero
    /// `border_width` shows up against the artboard.
    pub const OPAQUE_BLACK: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    /// Opaque white. Useful as a default tint (identity pass-through).
    pub const OPAQUE_WHITE: Color = Color {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };
}

// ---------------------------------------------------------------------------
// ComponentPayload — tagged enum of every component type
// ---------------------------------------------------------------------------

/// Tagged enum of every component type the protocol defines.
///
/// Adjacent tagging (`tag + content`) so each variant serializes as
/// `{ "type": "<kind>", "data": <Props> }`. This is the form every
/// consumer and producer sees on disk.
///
/// Adding a new component = adding a variant here + a default-factory
/// + the consumer-side mapping. The outer `WidgetNode` shape does not
/// change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "lowercase")]
pub enum ComponentPayload {
    /// Position + size + rotation + scale + flip.
    Transform(TransformProps),
    /// Background fill, border, corner radius.
    Style(StyleProps),
    /// Flex container layout (display, direction, gap, padding, ...).
    Layout(LayoutProps),
    /// Text content + style.
    Text(TextProps),
    /// Image path + tint + fit + slice.
    Image(ImageProps),
    /// User-interaction wiring (callbacks + pickability flags).
    Interaction(InteractionProps),
    /// Include reference to another `.beui` file.
    Include(IncludeProps),
}

impl ComponentPayload {
    /// Short type tag matching the `"type"` field on disk. Mirrors
    /// the map key used by `WidgetNode::components`.
    pub fn type_tag(&self) -> &'static str {
        match self {
            ComponentPayload::Transform(_) => "transform",
            ComponentPayload::Style(_) => "style",
            ComponentPayload::Layout(_) => "layout",
            ComponentPayload::Text(_) => "text",
            ComponentPayload::Image(_) => "image",
            ComponentPayload::Interaction(_) => "interaction",
            ComponentPayload::Include(_) => "include",
        }
    }
}

// ---------------------------------------------------------------------------
// Transform Component (every widget carries one)
// ---------------------------------------------------------------------------

/// Position + size + rotation + scale + flip + z-order. Every widget in
/// the tree MUST carry a `transform` component (enforced by `validate`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransformProps {
    /// Position in the parent's coordinate space.
    pub position: PositionVal,
    /// Width/height of the widget.
    pub size: SizeVal,
    /// Rotation in radians (positive = counter-clockwise).
    #[serde(default)]
    pub rotation: f32,
    /// Scale factors applied to the rendered content.
    #[serde(default)]
    pub scale: Scale,
    /// Axis-mirror flags for the content.
    #[serde(default)]
    pub flip: Flip,
    /// Painter's-algorithm stacking order within the parent's stack.
    /// Higher values paint on top of siblings. `attachChild` in the editor
    /// auto-bumps a child to `parent.z_index + 1` on attach when the child
    /// would otherwise paint at or below the parent (a "child behind its
    /// parent" footgun). Default 0 — the artboard root sits at z=0, so
    /// freshly-dropped widgets paint one layer above the artboard frame.
    #[serde(default)]
    pub z_index: i32,
}

impl Default for TransformProps {
    fn default() -> Self {
        Self {
            position: PositionVal {
                x: Val::px(0.0),
                y: Val::px(0.0),
            },
            size: SizeVal::default(),
            rotation: 0.0,
            scale: Scale::default(),
            flip: Flip::default(),
            z_index: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Style Component (every widget carries one)
// ---------------------------------------------------------------------------

/// Visual styling: background fill, border color + width, corner radius.
/// Every widget MUST carry a `style` component.
///
/// Master toggles (`fill_enabled`, `border_enabled`) default to `false`
/// so freshly-created widgets render with no fill or outline until the
/// user opts in. When `fill_enabled` is false the spawner skips the
/// `BackgroundColor` component entirely; same for borders.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StyleProps {
    /// Background fill color. Only rendered when `fill_enabled` is true.
    #[serde(default)]
    pub background: Color,
    /// Border outline color. Only rendered when `border_enabled` is true.
    #[serde(default = "default_border_color")]
    pub border_color: Color,
    /// Border outline width in pixels. Zero means no border rendered.
    #[serde(default)]
    pub border_width: f32,
    /// Border corner radius in pixels.
    #[serde(default)]
    pub border_radius: f32,
    /// Whether to render the background fill. Default `false` so
    /// freshly-created widgets show no fill until the user opts in.
    #[serde(default)]
    pub fill_enabled: bool,
    /// Whether to render the border. Default `false`.
    #[serde(default)]
    pub border_enabled: bool,
}

fn default_border_color() -> Color {
    Color::OPAQUE_BLACK
}

impl Default for StyleProps {
    fn default() -> Self {
        Self {
            background: Color::default(),
            border_color: Color::OPAQUE_BLACK,
            border_width: 0.0,
            border_radius: 0.0,
            fill_enabled: false,
            border_enabled: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Layout Component (flex container layout)
// ---------------------------------------------------------------------------

/// Flex container layout (mirrors Bevy's UI flex API). Optional — only
/// containers that need to lay out children need it. A widget without
/// a `layout` component has `position: absolute` semantics (its
/// `transform.position` is interpreted in the parent's coordinate
/// space).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct LayoutProps {
    /// Box model: flex or none.
    #[serde(default = "default_display")]
    pub display: Display,
    /// Main axis direction.
    #[serde(default = "default_flex_direction")]
    pub flex_direction: FlexDirection,
    /// Main-axis child alignment.
    #[serde(default)]
    pub justify_content: JustifyContent,
    /// Cross-axis child alignment.
    #[serde(default)]
    pub align_items: AlignItems,
    /// Gap between children in pixels.
    #[serde(default)]
    pub gap: f32,
    /// Inner padding (top/right/bottom/left).
    #[serde(default)]
    pub padding: Padding,
    /// Outer margin (top/right/bottom/left).
    #[serde(default)]
    pub margin: Margin,
    /// Flex shrink factor. `1.0` (the default) matches Bevy 0.19's
    /// taffy-backed `Node.flex_shrink` default. When a flex child
    /// has `flex_shrink > 0` and the container's main-axis content
    /// is too small to fit all children, taffy reduces the child's
    /// main extent proportionally. A `0` disables shrinking (children
    /// overflow the container instead of squishing).
    ///
    /// Phase 6 added this field; legacy assets without it parse to
    /// `1.0` (Bevy-compatible default) via `#[serde(default)]`.
    #[serde(default = "default_flex_shrink")]
    pub flex_shrink: f32,
    /// Optional absolute position. When `Some`, the widget is laid out
    /// at the given offset in the parent's content box (ignores flex).
    #[serde(default)]
    pub position: Option<AbsolutePosition>,
}

fn default_flex_shrink() -> f32 {
    1.0
}

fn default_display() -> Display {
    Display::Flex
}
fn default_flex_direction() -> FlexDirection {
    FlexDirection::Row
}

/// Box model: `Flex` lays out children according to flex rules; `None`
/// hides the widget and skips its subtree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Display {
    /// Flex container that lays out children.
    #[default]
    Flex,
    /// Not rendered (display: none).
    None,
}

/// Main axis direction for a flex container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum FlexDirection {
    /// Left to right.
    #[default]
    Row,
    /// Top to bottom.
    Column,
    /// Right to left.
    RowReverse,
    /// Bottom to top.
    ColumnReverse,
}

/// Main-axis child alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum JustifyContent {
    /// Pack children at the start of the main axis.
    #[default]
    FlexStart,
    /// Pack children at the end of the main axis.
    FlexEnd,
    /// Pack children at the center of the main axis.
    Center,
    /// First child at the start, last at the end, rest evenly distributed.
    SpaceBetween,
    /// Equal space around each child (half-gap at the edges).
    SpaceAround,
    /// Equal space around each child (full gap at the edges).
    SpaceEvenly,
}

/// Cross-axis child alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum AlignItems {
    /// Stretch children to fill the cross axis.
    #[default]
    Stretch,
    /// Align children to the cross-axis start.
    FlexStart,
    /// Align children to the cross-axis end.
    FlexEnd,
    /// Align children to the cross-axis center.
    Center,
}

/// Inner padding in CSS shorthand: top/right/bottom/left, all in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Padding {
    /// Top padding in pixels.
    pub top: f32,
    /// Right padding in pixels.
    pub right: f32,
    /// Bottom padding in pixels.
    pub bottom: f32,
    /// Left padding in pixels.
    pub left: f32,
}

/// Outer margin in CSS shorthand: top/right/bottom/left, all in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Margin {
    /// Top margin in pixels.
    pub top: f32,
    /// Right margin in pixels.
    pub right: f32,
    /// Bottom margin in pixels.
    pub bottom: f32,
    /// Left margin in pixels.
    pub left: f32,
}

/// Absolute position override. When set on a `LayoutProps`, the widget
/// is positioned at `(x, y)` in the parent's content box, ignoring flex.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct AbsolutePosition {
    /// Horizontal offset from the parent's content-box origin, in pixels.
    pub x: f32,
    /// Vertical offset from the parent's content-box origin, in pixels.
    pub y: f32,
}

// ---------------------------------------------------------------------------
// Text Component (Text/Button widgets)
// ---------------------------------------------------------------------------

/// Text content for a `Text` or `Button` widget. The editor shows this
/// in the Inspector; the Bevy spawner maps it to a `Text` component.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextProps {
    /// The string to render.
    pub content: String,
    /// Font size in points.
    pub font_size: f32,
    /// Text alignment (left/center/right).
    #[serde(default)]
    pub align: TextAlign,
    /// Text color.
    #[serde(default)]
    pub color: Color,
}

impl Default for TextProps {
    fn default() -> Self {
        Self {
            content: String::new(),
            font_size: 16.0,
            align: TextAlign::default(),
            color: Color::OPAQUE_BLACK,
        }
    }
}

/// Text alignment along the main axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextAlign {
    /// Left-aligned (default for LTR text).
    #[default]
    Left,
    /// Center-aligned.
    Center,
    /// Right-aligned.
    Right,
}

// ---------------------------------------------------------------------------
// Image Component (Image widgets)
// ---------------------------------------------------------------------------

/// Image payload: source path, multiplicative tint, fit / stretch mode,
/// and 9-patch slice insets.
///
/// - `path` is consumer-relative (e.g. `"ui/hero.png"`). An empty
///   string renders a transparent image (useful for staging).
/// - `tint_enabled = false` means Bevy uses its default `Color::WHITE`
///   pass-through; set to `true` for the spawner to attach
///   `ImageNode::color`.
/// - `fit` is an editor-friendly enum. The consumer maps the four
///   variants onto its own image-mode API.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageProps {
    /// Consumer-relative image path. Empty string renders transparent.
    #[serde(default)]
    pub path: String,
    /// Multiplicative tint color. Only applied when `tint_enabled` is true.
    #[serde(default = "default_image_tint")]
    pub tint: Color,
    /// Whether to apply the tint. Default `false` (identity pass-through).
    #[serde(default)]
    pub tint_enabled: bool,
    /// How the image fits its box.
    #[serde(default)]
    pub fit: ImageFit,
    /// 9-patch slice insets (left/top/right/bottom).
    #[serde(default)]
    pub slice_insets: SliceInsets,
    /// Whether to apply 9-patch slicing. Default `false`.
    #[serde(default)]
    pub slice_enabled: bool,
}

fn default_image_tint() -> Color {
    Color::OPAQUE_WHITE
}

impl Default for ImageProps {
    fn default() -> Self {
        Self {
            path: String::new(),
            tint: default_image_tint(),
            tint_enabled: false,
            fit: ImageFit::Stretch,
            slice_insets: SliceInsets::default(),
            slice_enabled: false,
        }
    }
}

/// How an image is rendered into its box. The consumer maps these
/// onto its own image-mode API (e.g. Bevy's `NodeImageMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFit {
    /// Aspect-distorting stretch to fill the box.
    #[default]
    Stretch,
    /// Cover (fills box, may crop). Maps to consumer's cover mode.
    Cover,
    /// Contain (fits inside, may letterbox). Maps to consumer's
    /// contain mode.
    Contain,
    /// Tiled repetition.
    Tile,
}

/// 9-patch slice insets: how far in from each edge the content
/// region starts. All values in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct SliceInsets {
    /// Left inset in pixels.
    pub left: f32,
    /// Top inset in pixels.
    pub top: f32,
    /// Right inset in pixels.
    pub right: f32,
    /// Bottom inset in pixels.
    pub bottom: f32,
}

// ---------------------------------------------------------------------------
// Interaction Component (any widget)
// ---------------------------------------------------------------------------

/// User-interaction wiring: click/hover/focus callbacks and
/// pickability/focusability flags. Optional — widgets that don't
/// carry it are pickable by default and have no handlers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InteractionProps {
    /// Callback identifier for click events. Resolved by the consumer
    /// (typically looked up in a sidecar-side handler table).
    #[serde(default)]
    pub onclick: Option<String>,
    /// Callback identifier for hover-enter events.
    #[serde(default)]
    pub onhover: Option<String>,
    /// Callback identifier for focus events.
    #[serde(default)]
    pub onfocus: Option<String>,
    /// Whether pointer events reach this widget. Defaults to true so
    /// a missing `interaction` component does NOT silently disable
    /// interaction.
    #[serde(default = "default_true")]
    pub pickable: bool,
    /// Whether keyboard focus can land here.
    #[serde(default)]
    pub focusable: bool,
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Include Component (Include widgets only)
// ---------------------------------------------------------------------------

/// Reference to another `.beui` file. The path is resolved by the
/// consumer relative to the asset's containing folder, then to the
/// workspace root. `Include` widgets MUST carry this component
/// (enforced by `validate`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IncludeProps {
    /// Source path, e.g. `"_shared/menu-panel.beui"`.
    pub source: String,
}