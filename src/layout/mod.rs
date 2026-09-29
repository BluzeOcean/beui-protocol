//! Layout reference functions.
//!
//! Every consumer (the bevy-ui-editor's sidecar, the bevy-lab
//! playground, and any external Bevy project that loads `.beui`
//! files) MUST route its layout adapter through this module. The
//! reference math lives here ONCE; consumers wrap it with their
//! engine-specific types (e.g. Bevy 0.19's `bevy_ui::Node`).
//!
//! ## Hidden-ness invariant
//!
//! [`layout_to_flex`] NEVER returns `Display::None` and [`is_under_parent_layout`]
//! NEVER implies `Display::None` / `Visibility::Hidden` / `transform.size = (0,0)`.
//! Hidden-ness is a separate concern owned by the editor's overrides
//! store, NOT by the layout. Consumers MUST refuse to emit
//! `Display::None` or `Visibility::Hidden` based on `LayoutDownward`.
//!
//! ## Mental model
//!
//! - **Downward** ("how I lay out my children"). Authored as
//!   [`LayoutDownwardProps`]. The user picks a `LayoutType`
//!   (`None` / `Horizontal` / `Vertical` / `Grid`); the four flex-input
//!   fields (`flex_direction`, `justify_content`, `align_items`,
//!   `display`) are derived.
//! - **Upward** ("how I relate to my parent's flex flow"). Authored as
//!   [`LayoutUpwardProps`]. The user picks `Inherit` (default — child
//!   participates in parent's flow) or `Ignore` (child escapes flow
//!   and is positioned by its `transform`).
//! - **Parent gate.** Whether a child's `transform.position` is
//!   meaningful is determined by [`is_under_parent_layout`]: only
//!   children with `mode = Ignore` (or with a parent that has no flex
//!   layout) get absolute positioning; the rest are flex items.
//!
//! See `tests.rs` (or the inline `#[cfg(test)] mod tests` below) for
//! the reference test cases that pin the contract — any consumer
//! SHOULD mirror those tests on its side, comparing its actual
//! output against the function's output.

use crate::schema::{
    ComponentPayload, Display, FlexDirection, FlexInputs, LayoutDownwardProps, LayoutType,
    LayoutUpwardMode, LayoutUpwardProps, WidgetNode,
};

/// Project a [`LayoutDownwardProps`] into a [`FlexInputs`] bundle.
///
/// **`layout_to_flex` always returns `Display::Flex`** — this is the
/// hidden-ness invariant. `LayoutType::None` is a layout TYPE but does
/// NOT hide the widget; an empty flex container is still a valid
/// container that draws itself (background, borders) and renders its
/// children positioned by their `transform`.
///
/// Mapping by [`LayoutType`]:
///
/// - [`LayoutType::None`]    → `Flex { Row, FlexStart, Stretch }`
///                             (always Flex per invariant; user's
///                             justify/align/direction are ignored here
///                             and overridden to safe defaults)
/// - [`LayoutType::Horizontal`] → `Flex { Row, user_justify, user_align }`
/// - [`LayoutType::Vertical`]   → `Flex { Column, user_justify, user_align }`
/// - [`LayoutType::Grid`]       → `Flex { Row, user_justify, user_align }`
///                                 (placeholder for v2; emits Horizontal
///                                 math until the v2 grid solver ships)
///
/// Spacing (gap / padding / margin) and `flex_shrink` are NOT in
/// [`FlexInputs`] — consumers apply them to the engine node directly
/// from the input props.
pub fn layout_to_flex(down: &LayoutDownwardProps) -> FlexInputs {
    // The hidden-ness invariant: always Flex. If a future contributor
    // adds a branch that returns Display::None, the
    // `hiddenness_invariant_never_returns_none` test below will catch
    // it. See CONSUMER_COMPLIANCE.md.
    let display = Display::Flex;
    match down.type_ {
        LayoutType::None => FlexInputs {
            display,
            flex_direction: FlexDirection::Row,
            justify_content: down.justify_content,
            align_items: down.align_items,
        },
        LayoutType::Horizontal => FlexInputs {
            display,
            flex_direction: FlexDirection::Row,
            justify_content: down.justify_content,
            align_items: down.align_items,
        },
        LayoutType::Vertical => FlexInputs {
            display,
            flex_direction: FlexDirection::Column,
            justify_content: down.justify_content,
            align_items: down.align_items,
        },
        LayoutType::Grid => FlexInputs {
            // v2 placeholder: Horizontal math.
            display,
            flex_direction: FlexDirection::Row,
            justify_content: down.justify_content,
            align_items: down.align_items,
        },
    }
}

/// Decide whether a child participates in its parent's flex flow.
///
/// The decision logic:
/// 1. If `parent` is `None` (child is at the root level), return `false`.
/// 2. If the child carries a [`LayoutUpwardProps`] with [`mode = Ignore`](LayoutUpwardMode::Ignore),
///    return `false` (child escapes the flow).
/// 3. If the parent has no `LayoutDownward` payload, treat its type as
///    [`LayoutType::None`] and return `false`.
/// 4. If the parent's type is [`LayoutType::Horizontal`],
///    [`LayoutType::Vertical`], or [`LayoutType::Grid`], return `true`.
///
/// Absence of the upward component on the child is equivalent to
/// `mode = Inherit` (the struct's `Default`).
///
/// **Note**: this function is read-only — it does not mutate `child`
/// or `parent` and does not look at the child's `transform`.
pub fn is_under_parent_layout(child: &WidgetNode, parent: Option<&WidgetNode>) -> bool {
    let Some(parent) = parent else {
        return false;
    };
    // 2. upward override
    if let Some(up) = extract_layout_upward(child) {
        if up.mode == LayoutUpwardMode::Ignore {
            return false;
        }
    }
    // 3 + 4. parent gate
    let layout_type = extract_layout_downward(parent)
        .map(|d| d.type_)
        .unwrap_or(LayoutType::None);
    matches!(
        layout_type,
        LayoutType::Horizontal | LayoutType::Vertical | LayoutType::Grid
    )
}

/// Extract a borrowed `LayoutDownward` payload if the node carries
/// one (under the `"layout_downward"` component key).
///
/// Returns `None` if the node has no `"layout_downward"` entry in its
/// `components` map. Other component keys (`layout_upward`, transform,
/// style, ...) are ignored.
pub fn extract_layout_downward(node: &WidgetNode) -> Option<&LayoutDownwardProps> {
    match node.components.get("layout_downward") {
        Some(ComponentPayload::LayoutDownward(d)) => Some(d),
        _ => None,
    }
}

/// Extract a borrowed `LayoutUpward` payload if the node carries one
/// (under the `"layout_upward"` component key).
///
/// Returns `None` if the node has no `"layout_upward"` entry. A missing
/// entry is semantically equivalent to `LayoutUpwardProps { mode:
/// Inherit }` — the default.
pub fn extract_layout_upward(node: &WidgetNode) -> Option<&LayoutUpwardProps> {
    match node.components.get("layout_upward") {
        Some(ComponentPayload::LayoutUpward(u)) => Some(u),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests — these pin the contract. External consumers SHOULD mirror
// them on their side and compare their own output to the function's
// output for the same input.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{ComponentPayload, LayoutUpwardMode, WidgetKind};
    use std::collections::BTreeMap;

    /// Build a minimal widget node carrying only `transform` + `style`
    /// plus an optional `"layout_downward"` + `"layout_upward"` entry
    /// constructed by the caller. Everything else is irrelevant to
    /// the layout functions under test.
    fn node_with(down: Option<LayoutDownwardProps>, up: Option<LayoutUpwardProps>) -> WidgetNode {
        let mut components: BTreeMap<String, ComponentPayload> = BTreeMap::new();
        if let Some(d) = down {
            components.insert("layout_downward".into(), ComponentPayload::LayoutDownward(d));
        }
        if let Some(u) = up {
            components.insert("layout_upward".into(), ComponentPayload::LayoutUpward(u));
        }
        WidgetNode {
            id: "n".into(),
            name: None,
            is_root: false,
            kind: WidgetKind::Container,
            children: vec![],
            components,
        }
    }

    // ----- layout_to_flex: cover all 4 LayoutType × key field combos

    /// Helper: assemble a downward props with a given type and the
    /// shape-default for everything else.
    fn down(type_: LayoutType) -> LayoutDownwardProps {
        LayoutDownwardProps {
            type_,
            ..LayoutDownwardProps::default()
        }
    }

    #[test]
    fn layout_to_flex_none_returns_flex_with_row_and_user_justify_align() {
        let flex = layout_to_flex(&down(LayoutType::None));
        // HIDDEN-NESS INVARIANT: always Flex, never None.
        assert!(matches!(flex.display, Display::Flex));
        assert!(matches!(flex.flex_direction, FlexDirection::Row));
        // justify_content / align_items pass through unchanged for None
        // (the user's authored values are still applied).
        assert!(matches!(
            flex.justify_content,
            crate::JustifyContent::FlexStart
        ));
        assert!(matches!(flex.align_items, crate::AlignItems::Stretch));
    }

    #[test]
    fn layout_to_flex_horizontal_returns_flex_with_row() {
        let flex = layout_to_flex(&down(LayoutType::Horizontal));
        assert!(matches!(flex.display, Display::Flex));
        assert!(matches!(flex.flex_direction, FlexDirection::Row));
    }

    #[test]
    fn layout_to_flex_vertical_returns_flex_with_column() {
        let flex = layout_to_flex(&down(LayoutType::Vertical));
        assert!(matches!(flex.display, Display::Flex));
        assert!(matches!(flex.flex_direction, FlexDirection::Column));
    }

    /// Grid is a v2 placeholder that uses Horizontal math in v1 so
    /// user assets round-trip cleanly before v2 lands.
    #[test]
    fn layout_to_flex_grid_returns_flex_with_row_placeholder() {
        let flex = layout_to_flex(&down(LayoutType::Grid));
        assert!(matches!(flex.display, Display::Flex));
        // v2 will overwrite this; the test pin locks the placeholder.
        assert!(matches!(flex.flex_direction, FlexDirection::Row));
    }

    /// Cross-product exhaustiveness: 4 LayoutType variants × the four
    /// default FlexInputs fields. The loop proves every path in
    /// `layout_to_flex`'s match is reachable.
    #[test]
    fn layout_to_flex_all_layout_types_covered() {
        for ty in [
            LayoutType::None,
            LayoutType::Horizontal,
            LayoutType::Vertical,
            LayoutType::Grid,
        ] {
            let flex = layout_to_flex(&down(ty));
            // every LayoutType produces Display::Flex (the hidden-ness
            // invariant). This is the load-bearing assertion.
            assert!(
                matches!(flex.display, Display::Flex),
                "{ty:?} produced Display::None — hidden-ness invariant broken"
            );
        }
    }

    /// The hidden-ness invariant, called out separately so a broken
    /// layout branch produces a glaring failure rather than being
    /// absorbed into a generic test.
    #[test]
    fn hiddenness_invariant_never_returns_none() {
        // Iterate every LayoutType AND every default FlexInputs the
        // user could have authored. For each combination, display must
        // be Flex.
        for ty in [
            LayoutType::None,
            LayoutType::Horizontal,
            LayoutType::Vertical,
            LayoutType::Grid,
        ] {
            for jc in [
                crate::JustifyContent::FlexStart,
                crate::JustifyContent::FlexEnd,
                crate::JustifyContent::Center,
                crate::JustifyContent::SpaceBetween,
                crate::JustifyContent::SpaceAround,
                crate::JustifyContent::SpaceEvenly,
            ] {
                for ai in [
                    crate::AlignItems::Stretch,
                    crate::AlignItems::FlexStart,
                    crate::AlignItems::FlexEnd,
                    crate::AlignItems::Center,
                ] {
                    let d = LayoutDownwardProps {
                        type_: ty,
                        justify_content: jc,
                        align_items: ai,
                        ..LayoutDownwardProps::default()
                    };
                    let flex = layout_to_flex(&d);
                    assert!(
                        matches!(flex.display, Display::Flex),
                        "({ty:?}, {jc:?}, {ai:?}) produced {:?}",
                        flex.display
                    );
                }
            }
        }
    }

    /// User-authored justify_content / align_items survive the
    /// `Horizontal` branch unchanged.
    #[test]
    fn layout_to_flex_preserves_user_justify_and_align_for_horizontal() {
        let d = LayoutDownwardProps {
            type_: LayoutType::Horizontal,
            flex_direction: FlexDirection::Row,
            justify_content: crate::JustifyContent::SpaceBetween,
            align_items: crate::AlignItems::Center,
            gap: 8.0,
            ..LayoutDownwardProps::default()
        };
        let flex = layout_to_flex(&d);
        assert!(matches!(
            flex.justify_content,
            crate::JustifyContent::SpaceBetween
        ));
        assert!(matches!(flex.align_items, crate::AlignItems::Center));
    }

    // ----- is_under_parent_layout

    #[test]
    fn no_parent_means_not_under_layout() {
        let child = node_with(None, None);
        assert!(!is_under_parent_layout(&child, None));
    }

    /// Cross-product: 4 LayoutType on parent × 2 LayoutUpwardMode on
    /// child × null parent (the no-parent case is already covered).
    #[test]
    fn is_under_parent_layout_inherit_and_any_non_none_parent_type() {
        for parent_type in [
            LayoutType::None,
            LayoutType::Horizontal,
            LayoutType::Vertical,
            LayoutType::Grid,
        ] {
            for mode in [LayoutUpwardMode::Inherit, LayoutUpwardMode::Ignore] {
                let child = node_with(None, Some(LayoutUpwardProps { mode }));
                let parent = node_with(Some(down(parent_type)), None);
                let expected = mode == LayoutUpwardMode::Inherit
                    && matches!(
                        parent_type,
                        LayoutType::Horizontal | LayoutType::Vertical | LayoutType::Grid
                    );
                assert_eq!(
                    is_under_parent_layout(&child, Some(&parent)),
                    expected,
                    "parent={parent_type:?} child_mode={mode:?} expected={expected}"
                );
            }
        }
    }

    /// Absence of upward component on the child is equivalent to
    /// `Inherit`.
    #[test]
    fn is_under_parent_layout_no_upward_means_inherit() {
        let child = node_with(None, None);
        for parent_type in [
            LayoutType::Horizontal,
            LayoutType::Vertical,
            LayoutType::Grid,
        ] {
            let parent = node_with(Some(down(parent_type)), None);
            assert!(
                is_under_parent_layout(&child, Some(&parent)),
                "{parent_type:?} without upward on child should be under layout"
            );
        }
    }

    /// Parent with no downward payload at all is treated as type None
    /// — child is not under layout regardless of upward.
    #[test]
    fn is_under_parent_layout_parent_without_downward_is_type_none() {
        let child = node_with(None, None);
        let parent = node_with(None, None);
        assert!(!is_under_parent_layout(&child, Some(&parent)));
    }

    /// Ignore mode escapes EVERY parent's layout, even when the parent
    /// is Horizontal / Vertical / Grid.
    #[test]
    fn is_under_parent_layout_ignore_escapes_parent_layout() {
        let child = node_with(None, Some(LayoutUpwardProps { mode: LayoutUpwardMode::Ignore }));
        for parent_type in [
            LayoutType::Horizontal,
            LayoutType::Vertical,
            LayoutType::Grid,
        ] {
            let parent = node_with(Some(down(parent_type)), None);
            assert!(
                !is_under_parent_layout(&child, Some(&parent)),
                "Ignore should escape {parent_type:?}"
            );
        }
    }

    // ----- extractors

    #[test]
    fn extract_layout_downward_present() {
        let n = node_with(Some(down(LayoutType::Horizontal)), None);
        assert_eq!(extract_layout_downward(&n).unwrap().type_, LayoutType::Horizontal);
    }

    #[test]
    fn extract_layout_downward_absent() {
        let n = node_with(None, None);
        assert!(extract_layout_downward(&n).is_none());
    }

    #[test]
    fn extract_layout_upward_present() {
        let n = node_with(None, Some(LayoutUpwardProps { mode: LayoutUpwardMode::Ignore }));
        assert_eq!(
            extract_layout_upward(&n).unwrap().mode,
            LayoutUpwardMode::Ignore
        );
    }

    #[test]
    fn extract_layout_upward_absent() {
        let n = node_with(None, None);
        assert!(extract_layout_upward(&n).is_none());
    }
}
