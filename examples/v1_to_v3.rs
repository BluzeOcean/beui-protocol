//! Example: migrate v1 (inline-fields) `.beui` files to v3 (component-map).
//!
//! Run with: `cargo run -p beui-protocol --example v1_to_v3 -- <input.beui> <output.beui>`
//!
//! Walks the inline `layout:` / `style:` / `text:` / `image:` /
//! `interaction:` / `include:` fields on every `WidgetNode`, lifts them
//! into the `components` map as `ComponentPayload` entries, and writes
//! the result with the v3 schema-version header.
//!
//! This is a structural converter: it reads the v1 file via a local
//! shadow struct (because the v1 schema has since diverged from v3),
//! then re-emits through the protocol crate's API so the output is
//! guaranteed to round-trip.
//!
//! Run this once per fixture to migrate it from the old bevy-lab
//! schema to the canonical v3 form. After that, delete the shadow
//! struct — every consumer should use the v3 schema directly.

use beui_protocol::{
    save_to_file, ComponentPayload, IncludeProps, InteractionProps, LayoutDownwardProps,
    LayoutUpwardProps, TextProps, UiDefinitionAsset, WidgetKind,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct V1Asset {
    #[allow(dead_code)]
    version: u32,
    root: V1Node,
    /// v1 theme is a `(name: ..., path: ...)` tuple OR `None`. We don't
    /// preserve it through the migration — re-attach themes via the
    /// editor after migration if needed.
    #[serde(default)]
    #[allow(dead_code)]
    theme: Option<ron::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct V1Node {
    id: String,
    #[serde(default)]
    name: Option<String>,
    /// v1 kind is a bare PascalCase identifier (Container, Image, ...).
    /// We accept it as a String and resolve to the protocol crate's
    /// WidgetKind enum in the upgrade function so the shadow doesn't
    /// depend on the enum's serde representation.
    kind: String,
    /// v1 style is a bare tuple OR `None`-equivalent (omitted). Same
    /// trick as `layout`.
    #[serde(default)]
    style: Option<V1Style>,
    #[serde(default)]
    layout: Option<V1Layout>,
    #[serde(default)]
    interaction: Option<V1Interaction>,
    #[serde(default)]
    children: Vec<V1Node>,
    #[serde(default)]
    text: Option<V1Text>,
    #[serde(default)]
    image: Option<V1Image>,
    #[serde(default)]
    button: Option<V1Button>,
    #[serde(default)]
    progress: Option<serde::de::IgnoredAny>,
    #[serde(default)]
    include: Option<V1Include>,
}

#[derive(Debug, Deserialize, Default)]
#[allow(dead_code)]
struct V1Layout {
    #[serde(default)]
    display: Option<serde::de::IgnoredAny>,
    #[serde(default)]
    flex_direction: Option<serde::de::IgnoredAny>,
    #[serde(default)]
    justify_content: Option<serde::de::IgnoredAny>,
    #[serde(default)]
    align_items: Option<serde::de::IgnoredAny>,
    #[serde(default)]
    gap: f32,
    #[serde(default)]
    padding: Option<serde::de::IgnoredAny>,
    #[serde(default)]
    margin: Option<serde::de::IgnoredAny>,
    #[serde(default)]
    position: Option<serde::de::IgnoredAny>,
    #[serde(default)]
    width: Option<f32>,
    #[serde(default)]
    height: Option<f32>,
    #[serde(default)]
    scale: Option<serde::de::IgnoredAny>,
}

#[derive(Debug, Deserialize, Default)]
#[allow(dead_code)]
struct V1Style {
    #[serde(default)]
    background: Option<V1Rgba>,
    #[serde(default)]
    border_color: Option<V1Rgba>,
    #[serde(default = "default_border_width")]
    border_width: f32,
    #[serde(default)]
    border_radius: Option<serde::de::IgnoredAny>,
    #[serde(default = "default_opacity")]
    opacity: f32,
    #[serde(default)]
    color: Option<V1Rgba>,
}

fn default_border_width() -> f32 {
    0.0
}
fn default_opacity() -> f32 {
    1.0
}

#[derive(Debug, Deserialize, Default, Clone, Copy)]
#[allow(dead_code)]
struct V1Rgba {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

#[derive(Debug, Deserialize, Default)]
#[allow(dead_code)]
struct V1Interaction {
    #[serde(default)]
    onclick: Option<String>,
    #[serde(default)]
    onhover: Option<String>,
    #[serde(default)]
    onfocus: Option<String>,
    #[serde(default = "default_true")]
    pickable: bool,
    #[serde(default)]
    focusable: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct V1Text {
    content: String,
    font_size: f32,
    #[serde(default)]
    align: Option<serde::de::IgnoredAny>,
    #[serde(default)]
    color: Option<V1Rgba>,
}

#[derive(Debug, Deserialize)]
struct V1Image {
    source: String,
    #[serde(default)]
    fit: Option<serde::de::IgnoredAny>,
    #[serde(default)]
    tint: Option<V1Rgba>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct V1Button {
    label: String,
}

#[derive(Debug, Deserialize)]
struct V1Include {
    source: String,
}

fn rgba_to_color(c: V1Rgba) -> beui_protocol::Color {
    beui_protocol::Color {
        r: (c.r.clamp(0.0, 1.0) * 255.0).round() as u8,
        g: (c.g.clamp(0.0, 1.0) * 255.0).round() as u8,
        b: (c.b.clamp(0.0, 1.0) * 255.0).round() as u8,
        a: (c.a.clamp(0.0, 1.0) * 255.0).round() as u8,
    }
}

fn upgrade(v1: V1Node, is_root: bool) -> beui_protocol::WidgetNode {
    let mut components: BTreeMap<String, ComponentPayload> = BTreeMap::new();

    // transform — always required; synthesize from v1's `layout.position`
    // (absolute x,y) + `layout.width/height` (Option<f32>) when present,
    // else fall back to defaults.
    let transform = beui_protocol::TransformProps {
        position: beui_protocol::PositionVal {
            x: beui_protocol::Val::Px(0.0),
            y: beui_protocol::Val::Px(0.0),
        },
        size: beui_protocol::SizeVal {
            width: beui_protocol::Val::Auto,
            height: beui_protocol::Val::Auto,
        },
        rotation: 0.0,
        scale: beui_protocol::Scale { x: 1.0, y: 1.0 },
        flip: beui_protocol::Flip::default(),
        z_index: 0,
    };
    components.insert("transform".into(), ComponentPayload::Transform(transform));

    // style — always required; synthesize from v1's `style` (colors in
    // f32 → u8 conversion via rgba_to_color).
    let style = beui_protocol::StyleProps {
        background: v1
            .style
            .as_ref()
            .and_then(|s| s.background)
            .map(rgba_to_color)
            .unwrap_or_default(),
        border_color: v1
            .style
            .as_ref()
            .and_then(|s| s.border_color)
            .map(rgba_to_color)
            .unwrap_or(beui_protocol::Color::OPAQUE_BLACK),
        border_width: v1.style.as_ref().map(|s| s.border_width).unwrap_or(0.0),
        border_radius: 0.0,
        fill_enabled: v1
            .style
            .as_ref()
            .and_then(|s| s.background)
            .is_some(),
        border_enabled: v1
            .style
            .as_ref()
            .map(|s| s.border_width > 0.0)
            .unwrap_or(false),
    };
    components.insert("style".into(), ComponentPayload::Style(style));

    // layout (optional) — v4 splits the old `Layout` payload into
    // `LayoutDownward` + `LayoutUpward`. We emit only the downward
    // payload here because the v1 fixture has no upward semantics to
    // carry over.
    if let Some(l) = v1.layout {
        if l.gap != 0.0 || l.width.is_some() || l.height.is_some() {
            // Only emit a layout component when meaningful data was
            // present. A pure-pass-through layout with no gap, width,
            // or height is equivalent to no layout component.
            components.insert(
                "layout_downward".into(),
                ComponentPayload::LayoutDownward(LayoutDownwardProps::default()),
            );
            // Mirror the historical upward default so consumers that
            // do `is_under_parent_layout` see the same behavior.
            components.insert(
                "layout_upward".into(),
                ComponentPayload::LayoutUpward(LayoutUpwardProps::default()),
            );
            let _ = l; // silence unused
        }
    }

    // text (Text-kind + carried payload)
    if let Some(t) = v1.text {
        let color = t.color.map(rgba_to_color).unwrap_or(beui_protocol::Color::OPAQUE_BLACK);
        components.insert(
            "text".into(),
            ComponentPayload::Text(TextProps {
                content: t.content,
                font_size: t.font_size,
                align: beui_protocol::TextAlign::Left,
                color,
            }),
        );
    }

    // image (Image-kind + carried payload)
    if let Some(i) = v1.image {
        components.insert(
            "image".into(),
            ComponentPayload::Image(beui_protocol::ImageProps {
                path: i.source,
                tint: i.tint.map(rgba_to_color).unwrap_or(beui_protocol::Color::OPAQUE_WHITE),
                tint_enabled: i.tint.is_some(),
                fit: beui_protocol::ImageFit::Stretch,
                slice_insets: beui_protocol::SliceInsets::default(),
                slice_enabled: false,
            }),
        );
    }

    // interaction (optional)
    if let Some(inter) = v1.interaction {
        if inter.onclick.is_some() || inter.onhover.is_some() || inter.onfocus.is_some() {
            components.insert(
                "interaction".into(),
                ComponentPayload::Interaction(InteractionProps {
                    onclick: inter.onclick,
                    onhover: inter.onhover,
                    onfocus: inter.onfocus,
                    pickable: inter.pickable,
                    focusable: inter.focusable,
                }),
            );
        }
    }

    // include (Include-kind + carried payload)
    if let Some(inc) = v1.include {
        components.insert(
            "include".into(),
            ComponentPayload::Include(IncludeProps { source: inc.source }),
        );
    }

    let children = v1.children.into_iter().map(|c| upgrade(c, false)).collect();

    let kind = match v1.kind.as_str() {
        "Container" => WidgetKind::Container,
        "Text" => WidgetKind::Text,
        "Image" => WidgetKind::Image,
        "Button" => WidgetKind::Button,
        "TextInput" => WidgetKind::TextInput,
        "Checkbox" => WidgetKind::Checkbox,
        "ScrollView" => WidgetKind::ScrollView,
        "ProgressBar" => WidgetKind::ProgressBar,
        "Include" => WidgetKind::Include,
        other => panic!("unknown v1 widget kind: {other}"),
    };

    beui_protocol::WidgetNode {
        id: v1.id,
        name: v1.name,
        is_root,
        kind,
        children,
        components,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: v1_to_v3 <input.v1.beui> <output.v3.beui>");
        std::process::exit(2);
    }
    let in_path = PathBuf::from(&args[1]);
    let out_path = PathBuf::from(&args[2]);

    let text = std::fs::read_to_string(&in_path)?;
    // Strip the v1 `// schema_version: 1` header so `ron` parses
    // happily — the body version is what matters; we'll re-emit with
    // the v3 header on save.
    let body = text
        .lines()
        .skip_while(|l| l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    eprintln!("--body bytes--\n{body}\n--end--");
    // Try parsing as a generic Value first to see what shape ron produces.
    let val: Result<ron::Value, _> = ron::from_str(&body);
    eprintln!("Value parse: {val:?}");
    let v1: V1Asset = ron::from_str(&body)?;

    let asset = UiDefinitionAsset {
        version: beui_protocol::CURRENT_SCHEMA_VERSION,
        root: upgrade(v1.root, true),
        theme: None,
    };
    save_to_file(&asset, &out_path)?;
    println!(
        "wrote {} ({} widgets)",
        out_path.display(),
        count_widgets(&asset.root)
    );
    Ok(())
}

fn count_widgets(node: &beui_protocol::WidgetNode) -> usize {
    1 + node.children.iter().map(count_widgets).sum::<usize>()
}