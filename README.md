# beui-protocol

The `.beui` file protocol crate. The single source of truth for the
`.beui` file format shared between the **bevy-ui-editor** and any
external Bevy project that wants to author, load, or save UI assets.

> **Read [`PROTOCOL.md`](./PROTOCOL.md) for the full specification.**
> This README is the quick start.

## Install

### Path dependency (recommended for local development)

```toml
# your-project/Cargo.toml
[dependencies]
beui-protocol = { path = "C:/Users/Bluze/Local Crate/beui-protocol" }
```

### Git dependency (once you push the repo)

```toml
beui-protocol = { git = "https://github.com/bluzeocean/beui-protocol" }
```

### crates.io (once you publish)

```toml
beui-protocol = "0.3"
```

## Two-line usage

```rust
let asset = beui_protocol::load_from_file("ui/menu.beui")?;
beui_protocol::save_to_file(&asset, "ui/menu.out.beui")?;
```

## What you get

- **One canonical schema.** `UiDefinitionAsset`, `WidgetNode`,
  `WidgetKind`, `ComponentPayload`, plus every payload struct
  (`TransformProps`, `StyleProps`, `LayoutProps`, `TextProps`,
  `ImageProps`, `InteractionProps`, `IncludeProps`).
- **Version handshake on every load.** Refuses files newer than
  `CURRENT_SCHEMA_VERSION` (no silent partial-loads); migrates older
  files forward through the full chain.
- **Invariant validation on every load + save.** Catches duplicate
  ids, multiple roots, missing components per kind, etc. as
  structured `ProtocolError::InvariantViolation { kind, detail }`.
- **Atomic-ish file write.** `save_to_file` validates first,
  refuses on bad data, then writes.
- **Round-trip safety.** Every fixture passes
  `load → save → load` byte-stable.

## Run the test suite

```bash
cargo test
```

## Layout

```
beui-protocol/
├── Cargo.toml
├── PROTOCOL.md              # full specification (read this first)
├── README.md                # this file
├── src/
│   ├── lib.rs               # module wiring + crate-level docs
│   ├── error.rs             # ProtocolError + stable kind() strings
│   ├── schema.rs            # UiDefinitionAsset, WidgetNode, ComponentPayload, ...
│   ├── migrate.rs           # CURRENT_SCHEMA_VERSION + migrate() chain
│   ├── validate.rs          # invariant catalogue
│   └── io.rs                # load_from_*, save_to_*, round_trip, empty_asset
└── tests/
    └── round_trip.rs        # end-to-end fixtures across every version
```

## Adding a new component (the standard workflow)

1. Add a new variant to `ComponentPayload` in `src/schema.rs`.
2. Add a payload struct + `Default` impl in the same file.
3. Add a `match` arm in `ComponentPayload::type_tag()`.
4. If the new payload is **required** for some `WidgetKind`, add an
   invariant branch in `src/validate.rs` and update the catalogue in
   `PROTOCOL.md`.
5. Add a test in `tests/round_trip.rs` that the new payload
   round-trips.
6. If the new payload is part of a **new schema version**, bump
   `CURRENT_SCHEMA_VERSION` and add a migration arm.

## License

MIT OR Apache-2.0