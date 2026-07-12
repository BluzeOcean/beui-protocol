# The `.beui` File Protocol

**Status:** canonical specification, version 3.
**Audience:** anyone writing a producer or consumer for `.beui` files.
**Enforced by:** the `beui-protocol` Rust crate.

---

## 1. What this is

A `.beui` file is a RON-serialized UI tree. It is the wire format shared
between:

- the **bevy-ui-editor** (Tauri 2 + Svelte 5 desktop app — reads and
  writes),
- the **Bevy sidecar renderer** (`bevy-sidecar/` — reads, hot-reloads
  via `notify`),
- any **external Bevy project** that wants to ship pre-authored UI
  alongside game code (reads, optionally writes for tooling).

The protocol crate is the single source of truth: every consumer and
producer MUST use its public API. Consumers MUST NOT define their own
`UiDefinitionAsset` struct; producers MUST NOT hand-roll RON. The
crate owns versioning, validation, and the wire format.

---

## 2. Wire format

```
// schema_version: <N>
<ron body>
```

- **First line:** `// schema_version: N` where `N` is a `u32`. Pure
  metadata — RON's line-comment syntax makes it ignored by the parser,
  but the protocol requires it so producers and consumers agree on
  which version was intended.
- **Remainder:** a single RON value of type `UiDefinitionAsset` (see §5).
- **Encoding:** UTF-8, no BOM, LF or CRLF line endings both accepted.
- **Max size:** not enforced by the crate; consumers should impose
  their own cap if reading untrusted files.

A file without the header line still parses, but the protocol
**RECOMMENDS** every producer write it (the crate's `save_to_string`
always does) and every consumer verify it (the crate's `load_from_str`
always does).

---

## 3. Versioning rules

- `CURRENT_SCHEMA_VERSION = 3` (in `beui_protocol::migrate::CURRENT_SCHEMA_VERSION`).
- `MIN_SUPPORTED_VERSION = 1`.
- Every load goes through `migrate(asset)` before returning. The crate
  applies the migration chain forward until the asset reaches the
  current version.
- A file declaring a `version` newer than `CURRENT_SCHEMA_VERSION`
  returns `ProtocolError::AssetTooNew { file_version, supported }`.
  Callers MUST refuse and tell the user to upgrade — do NOT silently
  load fields they may not understand.
- A file declaring a `version` older than `MIN_SUPPORTED_VERSION`
  returns `ProtocolError::AssetTooOld`. Older files can be migrated
  forward; older-than-min cannot.
- Saving always writes with `version = CURRENT_SCHEMA_VERSION`.

### Bumping the protocol

1. Add `#[serde(default)]` to every new field on every existing struct
   so older assets deserialize cleanly into the new shape.
2. Add a migration arm in `migrate.rs` (e.g. `v3_to_v4`) that bumps
   the version and applies any structural reshape.
3. Bump `CURRENT_SCHEMA_VERSION`.
4. Add a round-trip test in `tests/round_trip.rs` covering the new
   version.
5. Bump the crate's `version` field in `Cargo.toml` (minor bump for
   additive changes, major for breaking).

---

## 4. Invariants

Every load validates the following after migration. The first
violation produces `ProtocolError::InvariantViolation { kind, detail }`
where `kind` is a stable string for programmatic consumers:

| `kind` string              | rule                                                  |
|----------------------------|-------------------------------------------------------|
| `wrong_version`            | `asset.version == CURRENT_SCHEMA_VERSION`.            |
| `multiple_roots`           | At most one widget in the tree has `is_root: true`.   |
| `duplicate_id`             | All widget ids in the tree are unique.                |
| `missing_transform`        | Every widget carries a `transform` component.         |
| `missing_style`            | Every widget carries a `style` component.             |
| `image_without_image`      | `kind: Image` widgets carry an `image` component.     |
| `include_without_include`  | `kind: Include` widgets carry an `include` component. |
| `text_without_text`        | `kind: Text` widgets carry a `text` component.        |

To add a new invariant: append a new `kind` string to the catalogue
above AND a new branch in `validate::validate_node`.

---

## 5. Type reference

```rust
// top-level asset
pub struct UiDefinitionAsset {
    pub version: u32,                  // CURRENT_SCHEMA_VERSION after migration
    pub root: WidgetNode,             // the single root widget
    pub theme: Option<ThemeRef>,      // optional theme reference
}

// a node in the tree
pub struct WidgetNode {
    pub id: String,                   // unique within the tree
    pub name: Option<String>,         // editor-only label; may be stripped on save
    pub is_root: bool,                // at most one per asset
    pub kind: WidgetKind,
    pub children: Vec<WidgetNode>,
    pub components: BTreeMap<String, ComponentPayload>,  // sorted, stable
}

// widget kinds (closed enum — add a new component payload, not a new kind)
pub enum WidgetKind {
    Container, Text, Image, Button, TextInput, Checkbox, ScrollView, ProgressBar, Include,
}

// component payloads (tagged enum; one variant per component type)
pub enum ComponentPayload {
    Transform(TransformProps),
    Style(StyleProps),
    Layout(LayoutProps),
    Text(TextProps),
    Image(ImageProps),
    Interaction(InteractionProps),
    Include(IncludeProps),
}
```

Full field-by-field docs: `cargo doc --open -p beui-protocol`.

---

## 6. Public API

```rust
use beui_protocol::{
    load_from_file, load_from_str, save_to_file, save_to_string,
    round_trip, empty_asset, validate,
    CURRENT_SCHEMA_VERSION, MIN_SUPPORTED_VERSION,
    ProtocolError, UiDefinitionAsset,
};

// Read
let asset: UiDefinitionAsset = load_from_file("ui/menu.beui")?;
let asset: UiDefinitionAsset = load_from_str(&ron_text)?;
let asset: UiDefinitionAsset = round_trip(&ron_text)?;  // load -> save -> load

// Write (refuses on invariant violation)
save_to_file(&asset, "ui/menu.out.beui")?;
let ron_text: String = save_to_string(&asset)?;

// Building
let asset: UiDefinitionAsset = empty_asset();

// Check without migrating
validate(&asset)?;

// Errors (every variant implements std::error::Error + Display)
match err.kind() {
    "not_utf8"            => /* tell user to re-save as UTF-8 */,
    "ron_parse"           => /* surface the offset to the user */,
    "asset_too_new"       => /* prompt to upgrade */,
    "asset_too_old"       => /* prompt to upgrade or accept the file */,
    "migration_failed"    => /* unrecoverable; show detail */,
    "bad_header"          => /* re-save from a known-good source */,
    "invariant_violation" => /* surface kind + detail */,
    "io"                  => /* show underlying io::Error */,
    _ => unreachable!(),
}
```

---

## 7. Color encoding

Colors are sRGB `u8` 0-255, four channels (`r, g, b, a`). Bevy
consumers convert at render time via
`Color::srgba(r/255.0, g/255.0, b/255.0, a/255.0)`. Alpha 0 = fully
transparent.

This is the single biggest source of cross-tool confusion: the
previous Bevy playground schema used `f32` 0..=1 colors. The v3
protocol standardizes on `u8`. If you have an old asset with `f32`
colors, multiply by 255 and round.

---

## 8. Includes

A node with `kind: Include` MUST carry an `include` component. The
`source` field is a path resolved by the consumer relative to the
asset's containing folder first, then to the workspace root. The
protocol does not interpret the path — that's the consumer's job.

When loading an `Include` node, the consumer is responsible for
loading the referenced file (also via `load_from_file`) and inlining
its tree at the position of the `Include` node. The protocol does
not resolve includes automatically — that's a render-time concern.

---

## 9. Failure mode policy

The protocol is strict. It refuses to load files it doesn't fully
understand, refuses to write files that violate invariants, and
surfaces every failure as a structured error with a stable `kind`
string. This is intentional: silent round-trips that lose data
(missing fields, dropped variants, lossy color conversion) are
worse than hard errors.

If you are a consumer that needs to load "anything you can" (e.g. a
diagnostic tool or a partial-implementation renderer), you can use
the lower-level `ron::from_str::<serde_json::Value>` path on the
file body and skip the protocol. But you lose every guarantee this
spec provides.

---

## 10. Adding this crate to your project

### Path dependency (recommended for local development)

```toml
# your-project/Cargo.toml
[dependencies]
beui-protocol = { path = "C:/Users/Bluze/Local Crate/beui-protocol" }
```

### Git dependency (after you push the repo)

```toml
beui-protocol = { git = "https://github.com/bluzeocean/beui-protocol" }
```

### crates.io (after you publish)

```toml
beui-protocol = "0.3"
```

### Minimal consumer example

```rust
use beui_protocol::load_from_file;

fn main() {
    let asset = load_from_file("ui/menu.beui").unwrap();
    println!("Loaded {} widgets in tree", count_widgets(&asset.root));
}

fn count_widgets(node: &beui_protocol::WidgetNode) -> usize {
    1 + node.children.iter().map(count_widgets).sum::<usize>()
}
```

### Bevy 0.19 consumer example (spawning the tree)

```rust
use bevy::prelude::*;
use beui_protocol::{load_from_file, ComponentPayload, WidgetKind};

fn spawn_ui_tree(asset: &beui_protocol::UiDefinitionAsset, parent: &mut ChildBuilder) {
    spawn_node(&asset.root, parent);
}

fn spawn_node(node: &beui_protocol::WidgetNode, parent: &mut ChildBuilder) {
    let mut cmd = parent.spawn(NodeBundle::default());
    if let Some(ComponentPayload::Image(img)) = node.components.get("image") {
        if !img.path.is_empty() {
            // map to bevy_ui::widget::image::ImageNode ...
        }
    }
    for child in &node.children {
        spawn_node(child, &mut cmd.commands().entity(cmd.target_entity()).into());
    }
}
```

---

## 11. Migration guide (consumers coming from older schemas)

### From v1 (Bevy playground inline-fields)

| v1 field         | v3 component       |
|------------------|--------------------|
| `layout: (...)`  | `components["layout"]`  = `ComponentPayload::Layout(...)` |
| `style: (...)`   | `components["style"]`   = `ComponentPayload::Style(...)`  |
| `text: Some((...))` | `components["text"]` = `ComponentPayload::Text(...)`     |
| `image: Some((...))` | `components["image"]` = `ComponentPayload::Image(...)`   |
| `include: Some((...))` | `components["include"]` = `ComponentPayload::Include(...)` |
| `interaction: (...)` | `components["interaction"]` = `ComponentPayload::Interaction(...)` |

Colors in v1 were `f32` 0..=1 (`Rgba`); multiply by 255 and round to
get v3's `u8` 0-255 form.

### From v2 (editor component-map, no Layout/Text/Interaction/Include)

The v2 schema is mostly a subset of v3. Migration lifts inline-able
fields into the new components and adds the `Include` variant to
`WidgetKind`. Existing v2 assets load with no data rewrite.

---

## 12. Why this exists

Before this crate, two schemas diverged:

- `shared/src/schema.rs` (the editor + sidecar) — `version: 2`,
  component-map layout.
- `bevy-lab/src/schema.rs` (the Bevy playground) — `version: 1`,
  inline-field layout.

Loading a bevy-lab file in the editor silently failed. Loading an
editor file in the playground silently failed. Neither was enforced
at the type level. Both projects patched their own copy of the
schema every time a field changed.

The `beui-protocol` crate is the unification: one type, one version,
one set of invariants, one load/save pipeline. Every producer and
every consumer links the same crate.

---

## Related

- `docs/superpowers/specs/2026-07-12-beui-file-format-authoring-guide.md`
  in the bevy-ui-editor repo — same protocol described from the
  authoring perspective.
- `memory/two-schemas-editor-and-bevy-lab.md` — the divergence this
  crate resolved.