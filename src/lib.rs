//! # beui-protocol
//!
//! The `.beui` file protocol crate. The single source of truth for the
//! `.beui` file format shared between the bevy-ui-editor and any
//! external Bevy project that wants to author, load, or save UI assets.
//!
//! Read [`PROTOCOL.md`](https://github.com/bluzeocean/beui-protocol/blob/main/PROTOCOL.md)
//! for the full specification. This crate-doc is a quick-reference.
//!
//! ## Quick start
//!
//! ```no_run
//! use std::path::Path;
//! use beui_protocol::{load_from_file, save_to_file};
//!
//! let asset = load_from_file(Path::new("ui/menu.beui")).unwrap();
//! save_to_file(&asset, Path::new("ui/menu.out.beui")).unwrap();
//! ```
//!
//! ## API surface
//!
//! ### Read
//! - [`load_from_file`] — read a `.beui` file from disk.
//! - [`load_from_str`] — read from an in-memory string.
//! - [`round_trip`] — load → save → load, returns the second-load result.
//!
//! ### Write
//! - [`save_to_file`] — atomic-ish write to disk.
//! - [`save_to_string`] — serialize to a RON string.
//!
//! ### Schema
//! - [`UiDefinitionAsset`] — the top-level asset.
//! - [`WidgetNode`] — a tree node with `components: { ... }`.
//! - [`WidgetKind`] — closed enum of widget kinds.
//! - [`ComponentPayload`] — tagged enum of every component type.
//! - [`TransformProps`], [`StyleProps`], [`LayoutProps`], [`TextProps`],
//!   [`ImageProps`], [`InteractionProps`], [`IncludeProps`] — the
//!   payload structs.
//!
//! ### Constants
//! - [`migrate::CURRENT_SCHEMA_VERSION`] — `3`.
//! - [`migrate::MIN_SUPPORTED_VERSION`] — `1`.
//!
//! ### Errors
//! - [`error::ProtocolError`] — structured failure modes.
//!   `ProtocolError::kind()` returns a stable string identifier.
//!
//! ## Pipeline (every load follows this)
//!
//! 1. Read the file as UTF-8 → [`ProtocolError::NotUtf8`] on bad bytes.
//! 2. Parse RON → [`ProtocolError::RonParse`] on syntax error.
//! 3. Verify the `// schema_version: N` header matches the body's
//!    `version:` field → [`ProtocolError::BadHeader`] on mismatch.
//! 4. [`migrate::migrate`] forward to `CURRENT_SCHEMA_VERSION` →
//!    [`ProtocolError::AssetTooNew`] / [`ProtocolError::AssetTooOld`]
//!    / [`ProtocolError::MigrationFailed`].
//! 5. [`validate::validate`] structural invariants →
//!    [`ProtocolError::InvariantViolation`].

#![warn(missing_docs)]
#![deny(rust_2018_idioms)]

pub mod error;
pub mod io;
pub mod migrate;
pub mod schema;
pub mod validate;

pub use error::ProtocolError;
pub use io::{
    empty_asset, load_from_file, load_from_str, round_trip, save_to_file, save_to_string,
};
pub use migrate::{CURRENT_SCHEMA_VERSION, MIN_SUPPORTED_VERSION};
pub use schema::{
    AbsolutePosition, AlignItems, Color, ComponentPayload, Display, Flip, FlexDirection,
    ImageFit, ImageProps, IncludeProps, InteractionProps, JustifyContent, LayoutProps, Margin,
    Padding, PositionVal, Scale, SizeVal, SliceInsets, StyleProps, TextAlign, TextProps,
    ThemeRef, TransformProps, UiDefinitionAsset, Val, WidgetKind, WidgetNode,
};
pub use validate::validate;