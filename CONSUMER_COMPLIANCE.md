# Consumer Compliance Guide

This document is the contract every consumer of `beui_protocol::layout`
MUST follow. It is the single source of truth for "what does it mean
to integrate the `.beui` file format into a Bevy project". The
reference functions in [`src/layout/mod.rs`](src/layout/mod.rs) +
the inline tests pinned under `beui_protocol::layout::tests` are the
mathematical contract; this document is the engineering contract.

## TL;DR

```toml
# Cargo.toml
[dependencies]
beui-protocol = "0.3"
```

```rust
use beui_protocol::layout::{layout_to_flex, is_under_parent_layout};
use beui_protocol::{load_from_file, LayoutDownwardProps, LayoutUpwardProps};
```

That's it. You do not need to import anything else from
`beui_protocol` to render a `.beui` file as Bevy nodes. **Do not
re-implement the layout math yourself.**

---

## The five rules

### 1. Import the reference functions

```rust
use beui_protocol::layout::{
    layout_to_flex,            // &LayoutDownwardProps -> FlexInputs
    is_under_parent_layout,   // (&WidgetNode, Option<&WidgetNode>) -> bool
    extract_layout_downward,  // &WidgetNode -> Option<&LayoutDownwardProps>
    extract_layout_upward,    // &WidgetNode -> Option<&LayoutUpwardProps>
};
```

These are the only functions you need. Every other layout decision
should defer to one of these.

### 2. Never invent layout math

If you find yourself writing `match layout.type_ { Horizontal => ...,
Vertical => ... }` directly in your adapter, STOP. Use
`layout_to_flex(&layout)` and let the protocol pick the
`flex_direction` for you. The four inputs in `FlexInputs`
(`display`, `flex_direction`, `justify_content`, `align_items`) are
the COMPLETE flex shape; the rest of `LayoutDownwardProps` (gap /
padding / margin / flex_shrink) flow through to Bevy as-is.

If you need a custom mapping (e.g. you want `Grid` to do something
different than the v1 placeholder), propose it as a `beui_protocol`
PR and add a test. Don't branch on `layout.type_` in your adapter.

### 3. Hidden-ness invariant — NEVER emit `Display::None` or `Visibility::Hidden` from layout

This is the most important rule.

`layout_to_flex` ALWAYS returns `Display::Flex`. There is no
`LayoutType` value that hides the widget — `LayoutType::None` is a
fully visible empty flex container. Hidden-ness is a separate concern
that lives in the editor's `editor-overrides-store.svelte.ts` (which
your consumer doesn't need to know about).

**What this means for your adapter:**

- Treat `Display::None` as a bug in the wire form. If a `.beui` file
  parses with `Display::None` somehow set on a `LayoutDownwardProps`,
  either (a) panic during asset validation, OR (b) coerce it to
  `Display::Flex` and continue. Never propagate `Display::None` to
  Bevy's `Node`.
- Never set `Visibility::Hidden` or `Visibility::Invisible` based on
  `LayoutDownward.type_` or `FlexInputs.display`.
- Never derive `transform.size = (0, 0)` from layout type. Size is
  the user's `transform.size`; layout doesn't touch it.
- Always render the widget's background and border (gated only by
  `StyleProps::fill_enabled` / `border_enabled`, which the editor
  controls separately).

A regression test for this rule: `hiddenness_invariant_never_returns_none`
in `beui_protocol::layout::tests`. Mirror that test in your adapter
crate to lock the contract on your side.

### 4. Use `is_under_parent_layout` to decide child layout strategy

Your adapter walks the `WidgetNode` tree and for each
(parent, child) pair decides whether the child is laid out by flex
or by absolute position:

```rust
for (parent, child) in walk_pairs(&asset.root) {
    if is_under_parent_layout(child, Some(parent)) {
        // child participates in the parent's flex flow. Apply flex
        // math to the parent's container; give the child
        // PositionType::Relative. The child's transform.size flows
        // through flex_basis; its transform.position is a Bevy
        // left/top offset FROM the flex-computed slot — never
        // ignored. See "Position-as-offset rule" below.
        let flex = layout_to_flex(&extract_layout_downward(parent).unwrap());
        // ... apply flex inputs to parent Bevy Node, give child PositionType::Relative + left/top
    } else {
        // child is positioned absolutely by its own transform. Apply
        // layout_to_flex to parent's downward props (if any) but
        // give child `PositionType::Absolute` so its transform.position
        // is interpreted in the parent's content box.
    }
}
```

Do not implement this decision yourself. `is_under_parent_layout`
encodes the parent gate (type ∈ {Horizontal, Vertical, Grid}) AND the
child upward override (`mode = Ignore`).

Note that for `Option<&WidgetNode>`, pass `None` for the root's
parent. Root children with no parent are never "under layout" in the
flex sense; they always render with their absolute transforms (unless
you opt to treat the artboard itself as a single Horizontal flex
container — that's a render-mode decision for your Bevy app, not the
protocol's).

#### 4a. Position-as-offset rule (in-flow children)

For an in-flow child (`is_under_parent_layout` returns `true`), the
adapter MUST honor `transform.position` as a Bevy `left/top` offset
FROM the flex-computed slot:

- `PositionType::Relative` + `left = transform.position.x` +
  `top = transform.position.y` is the standard CSS `position: relative`
  semantic in Bevy 0.19 — the offset is applied AFTER the flex
  solver places the slot.
- A child with `transform.position = (10, 0)` renders 10px right of
  its flex slot. A negative offset renders the child off the slot's
  leading edge. There is no clamping or clamping-to-slot behavior.
- `transform.size` still flows through `flex_basis` (NOT a render-time
  size); if you need a fixed size under flow, set the explicit size
  fields in `LayoutDownwardProps`. The editor's "position-only unlock"
  keeps `transform.size` + `transform.scale` inspector-locked under
  flow for this reason — Bevy would silently fight the parent's
  `align_items: Stretch`.
- This rule is the same in all three consumers: `bevy-sidecar`'s
  `widget_spawner`, `bevy-lab`'s `base_node`, and the editor's TS
  canvas-layout dispatcher. The `in_flow_child_keeps_left_top_as_flex_slot_offset`
  test in `bevy_ui_compat.rs` pins it on the bevy-sidecar side; mirror
  it in your adapter.

### 5. The reference tests pin the contract

`beui_protocol::layout::tests` covers all `LayoutType` ×
`LayoutUpwardMode` × null-parent combos plus the hidden-ness
invariant. If your adapter diverges from those tests on the same
fixture, either (a) you have a bug, OR (b) the protocol has shipped a
new version without you upgrading. Mirror those tests in your
adapter's test suite:

- `layout_to_flex_none_returns_flex_with_row_and_user_justify_align`
- `layout_to_flex_horizontal_returns_flex_with_row`
- `layout_to_flex_vertical_returns_flex_with_column`
- `layout_to_flex_grid_returns_flex_with_row_placeholder`
- `layout_to_flex_all_layout_types_covered`
- `hiddenness_invariant_never_returns_none`
- `is_under_parent_layout_inherit_and_any_non_none_parent_type`
- `is_under_parent_layout_no_upward_means_inherit`
- `is_under_parent_layout_parent_without_downward_is_type_none`
- `is_under_parent_layout_ignore_escapes_parent_layout`

The mirror should produce identical results to the protocol
function. If it doesn't, that's a divergence.

---

## What the protocol does NOT do

The protocol crate is the **schema + reference math**. It does not:

- Spawn Bevy entities. That's `bevy-sidecar` (editor preview) or
  your Bevy app.
- Render anything. Consumers translate `FlexInputs` into Bevy
  `Node` fields.
- Decide what's on-screen for the root. The artboard is a
  render-mode decision; the protocol just provides the data.
- Know about Bevy `Visibility`, focus, accessibility, or animation.
  Those are consumer concerns.

## What the protocol DOES do

- Defines the wire form (`LayoutDownward`, `LayoutUpward` payloads).
- Provides the reference functions (`layout_to_flex`,
  `is_under_parent_layout`).
- Pins the hidden-ness invariant via test
  (`hiddenness_invariant_never_returns_none`).

That's the contract. Stick to it and your consumer will produce the
same visual output as the editor's preview for the same `.beui`
file.
