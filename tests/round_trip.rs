//! End-to-end round-trip tests across every supported schema version.
//!
//! These tests guard the wire-format guarantee: every `.beui` file the
//! protocol can produce must round-trip cleanly through load → save →
//! load without data loss. If you bump `CURRENT_SCHEMA_VERSION`, add a
//! new fixture to the array below and a new assertion.

use beui_protocol::{
    empty_asset, load_from_str, round_trip, save_to_string, Color, ComponentPayload,
    CURRENT_SCHEMA_VERSION, ImageFit, ImageProps, IncludeProps, InteractionProps, LayoutProps,
    StyleProps, TextProps, TransformProps, UiDefinitionAsset, Val, WidgetKind, WidgetNode,
};
use std::collections::BTreeMap;

fn painted(asset: &mut UiDefinitionAsset) {
    let mut child_components = BTreeMap::new();
    child_components.insert(
        "transform".into(),
        ComponentPayload::Transform(TransformProps {
            position: beui_protocol::PositionVal {
                x: Val::Px(50.0),
                y: Val::Px(20.0),
            },
            size: beui_protocol::SizeVal {
                width: Val::Px(200.0),
                height: Val::Px(40.0),
            },
            rotation: 15.0,
            scale: beui_protocol::Scale { x: 2.0, y: 1.0 },
            flip: beui_protocol::Flip { x: false, y: true },
        }),
    );
    child_components.insert(
        "style".into(),
        ComponentPayload::Style(StyleProps {
            background: Color {
                r: 100,
                g: 50,
                b: 200,
                a: 255,
            },
            border_color: Color::OPAQUE_BLACK,
            border_width: 2.0,
            border_radius: 4.0,
            fill_enabled: true,
            border_enabled: true,
        }),
    );
    asset.root.children.push(WidgetNode {
        id: "btn".into(),
        name: Some("Play Button".into()),
        is_root: false,
        kind: WidgetKind::Button,
        children: vec![],
        components: child_components,
    });
}

#[test]
fn empty_asset_round_trips() {
    let original = empty_asset();
    let text = save_to_string(&original).unwrap();
    let reloaded = load_from_str(&text).unwrap();
    assert_eq!(original, reloaded);
}

#[test]
fn painted_asset_round_trips() {
    let mut original = empty_asset();
    painted(&mut original);
    let text = save_to_string(&original).unwrap();
    let reloaded = load_from_str(&text).unwrap();
    assert_eq!(original, reloaded);
}

#[test]
fn header_byte_sequence_is_stable() {
    let text = save_to_string(&empty_asset()).unwrap();
    let expected = format!("// schema_version: {CURRENT_SCHEMA_VERSION}\n");
    assert!(
        text.starts_with(&expected),
        "saved text must start with `{expected}`; got start: {:?}",
        &text[..text.len().min(40)]
    );
}

#[test]
fn helper_round_trip_matches_load_save_load() {
    let mut a = empty_asset();
    painted(&mut a);
    let text = save_to_string(&a).unwrap();
    let via_helper = round_trip(&text).unwrap();
    let via_manual = {
        let loaded = load_from_str(&text).unwrap();
        let text2 = save_to_string(&loaded).unwrap();
        load_from_str(&text2).unwrap()
    };
    assert_eq!(via_helper, via_manual);
}

#[test]
fn all_component_payload_variants_deserialize() {
    // Verify every component in the tagged enum round-trips by
    // constructing one of each, saving, and reloading.
    let mut components = BTreeMap::new();
    components.insert(
        "transform".into(),
        ComponentPayload::Transform(TransformProps::default()),
    );
    components.insert(
        "style".into(),
        ComponentPayload::Style(StyleProps::default()),
    );
    components.insert(
        "layout".into(),
        ComponentPayload::Layout(LayoutProps::default()),
    );
    components.insert(
        "text".into(),
        ComponentPayload::Text(TextProps::default()),
    );
    components.insert(
        "image".into(),
        ComponentPayload::Image(ImageProps {
            path: "ui/test.png".into(),
            fit: ImageFit::Cover,
            ..ImageProps::default()
        }),
    );
    components.insert(
        "interaction".into(),
        ComponentPayload::Interaction(InteractionProps {
            onclick: Some("play".into()),
            focusable: true,
            ..InteractionProps::default()
        }),
    );
    components.insert(
        "include".into(),
        ComponentPayload::Include(IncludeProps {
            source: "_shared/menu.beui".into(),
        }),
    );

    let asset = UiDefinitionAsset {
        version: CURRENT_SCHEMA_VERSION,
        root: WidgetNode {
            id: "root".into(),
            name: None,
            is_root: true,
            kind: WidgetKind::Container,
            children: vec![],
            components,
        },
        theme: None,
    };
    let text = save_to_string(&asset).unwrap();
    let reloaded = load_from_str(&text).unwrap();
    assert_eq!(asset, reloaded);
}

#[test]
fn widget_kind_include_round_trips() {
    let mut components = BTreeMap::new();
    components.insert(
        "transform".into(),
        ComponentPayload::Transform(TransformProps::default()),
    );
    components.insert(
        "style".into(),
        ComponentPayload::Style(StyleProps::default()),
    );
    components.insert(
        "include".into(),
        ComponentPayload::Include(IncludeProps {
            source: "_shared/menu.beui".into(),
        }),
    );
    let asset = UiDefinitionAsset {
        version: CURRENT_SCHEMA_VERSION,
        root: WidgetNode {
            id: "menu-ref".into(),
            name: None,
            is_root: true,
            kind: WidgetKind::Include,
            children: vec![],
            components,
        },
        theme: None,
    };
    let text = save_to_string(&asset).unwrap();
    let reloaded = load_from_str(&text).unwrap();
    assert_eq!(asset, reloaded);
}