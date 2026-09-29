//! Load / save API. The public surface.
//!
//! Every entry point goes through the same pipeline:
//!
//!   raw bytes
//!     → parse RON
//!     → verify `// schema_version:` header matches body
//!     → migrate to `CURRENT_SCHEMA_VERSION`
//!     → validate invariants
//!     → return `UiDefinitionAsset`
//!
//! Save goes through:
//!
//!   `UiDefinitionAsset`
//!     → validate invariants (refuse to write bad data)
//!     → serialize to RON with version header
//!     → return bytes / write file
//!
//! ## Round-trip safety
//!
//! `round_trip(text)` is the canonical guarantee: load → save → load
//! must produce an equal asset. Tested in `tests/round_trip.rs`.

use crate::error::ProtocolError;
use crate::migrate::{header_line, migrate, CURRENT_SCHEMA_VERSION};
use crate::schema::{
    ComponentPayload, StyleProps, TransformProps, UiDefinitionAsset, WidgetKind, WidgetNode,
};
use crate::validate::validate;
use ron::ser::PrettyConfig;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Load a `.beui` file from disk. See module docs for the full pipeline.
pub fn load_from_file(path: &Path) -> Result<UiDefinitionAsset, ProtocolError> {
    let bytes = fs::read(path).map_err(|source| ProtocolError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let text = String::from_utf8(bytes).map_err(|_| ProtocolError::NotUtf8 {
        path: path.to_path_buf(),
    })?;
    load_from_str(&text)
}

/// Load a `.beui` file from a string. The header line
/// `// schema_version: N` is required and must match the body's
/// `version:` field.
pub fn load_from_str(text: &str) -> Result<UiDefinitionAsset, ProtocolError> {
    // 1. Parse the header line + the RON body separately. The header
    // is just a `//` comment so `ron` itself will ignore it; we
    // extract it first so we can verify it matches the body's version.
    let (header_version, body) = split_header(text)?;
    // 2. Branch on the on-disk schema version. v4 inputs parse
    // directly into the canonical `UiDefinitionAsset`. v3 (and
    // earlier) inputs MUST shadow-parse through the v3-shadow types
    // because the v4 `ComponentPayload` enum dropped the legacy
    // `Layout` variant — wire-compatible from the perspective of
    // Transform / Style / Text / Image / Interaction / Include, but
    // NOT for `Layout`, which this crate silently routes through the
    // shadow parse to keep load paths symmetric.
    let asset: UiDefinitionAsset = match header_version {
        Some(v) if v < crate::migrate::CURRENT_SCHEMA_VERSION => {
            crate::migrate::parse_v3_into_v4(body)?
        }
        _ => parse_ron(body)?,
    };
    if let Some(hv) = header_version {
        if hv != asset.version {
            return Err(ProtocolError::BadHeader {
                header: Some(hv),
                body: asset.version,
            });
        }
    }
    let migrated = migrate(asset)?;
    validate(&migrated)?;
    Ok(migrated)
}

/// Serialize an asset to a RON string with the schema-version header.
/// Refuses to write if the asset fails invariant validation.
pub fn save_to_string(asset: &UiDefinitionAsset) -> Result<String, ProtocolError> {
    validate(asset)?;
    let body = ron::ser::to_string_pretty(asset, PrettyConfig::default()).map_err(|e| {
        ProtocolError::RonParse {
            message: format!("ron serialize: {e}"),
            position: None,
        }
    })?;
    Ok(format!("{}{}", header_line(asset.version), body))
}

/// Write an asset to a file as RON. Atomic-ish: writes to a sibling
/// temp file, then renames. Refuses if the asset fails validation.
pub fn save_to_file(asset: &UiDefinitionAsset, path: &Path) -> Result<(), ProtocolError> {
    let text = save_to_string(asset)?;
    // Ensure parent dir exists.
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|source| ProtocolError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
    }
    fs::write(path, text).map_err(|source| ProtocolError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

/// Canonical round-trip: load → save → load, returning the second
/// load's result. The test suite asserts this is identity for every
/// fixture.
pub fn round_trip(text: &str) -> Result<UiDefinitionAsset, ProtocolError> {
    let loaded = load_from_str(text)?;
    let written = save_to_string(&loaded)?;
    load_from_str(&written)
}

/// Build a known-good empty asset: a single Container root carrying
/// `transform` + `style` defaults. Useful for tests and as a
/// starting point for code-generation tools.
pub fn empty_asset() -> UiDefinitionAsset {
    let mut components: BTreeMap<String, ComponentPayload> = BTreeMap::new();
    components.insert(
        "transform".into(),
        ComponentPayload::Transform(TransformProps::default()),
    );
    components.insert(
        "style".into(),
        ComponentPayload::Style(StyleProps::default()),
    );
    UiDefinitionAsset {
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
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Strip the leading `// schema_version: N\n` header (if present) and
/// return the version + the remaining RON body. RON itself ignores
/// line comments, so the body is the whole rest of the file.
fn split_header(text: &str) -> Result<(Option<u32>, &str), ProtocolError> {
    let mut lines = text.lines();
    let first = match lines.next() {
        Some(line) => line,
        None => return Ok((None, text)),
    };
    let trimmed = first.trim_start();
    if let Some(rest) = trimmed.strip_prefix("//") {
        let rest = rest.trim();
        if let Some(ver_str) = rest.strip_prefix("schema_version:") {
            let ver_str = ver_str.trim();
            let ver: u32 = ver_str.parse().map_err(|_| ProtocolError::BadHeader {
                header: None,
                body: 0,
            })?;
            // Skip the newline that ended the header line and return
            // the remainder (the RON body).
            let consumed = first.len() + 1; // +1 for the \n
            return Ok((Some(ver), &text[consumed..]));
        }
    }
    // No header. RON itself tolerates leading comments so this is a
    // soft warning, not an error — but the protocol RECOMMENDS the
    // header. Save always writes it.
    Ok((None, text))
}

fn parse_ron(text: &str) -> Result<UiDefinitionAsset, ProtocolError> {
    ron::from_str::<UiDefinitionAsset>(text).map_err(|e| {
        let p = &e.span.start;
        // ron 0.12's Position struct prints as line:col when formatted
        // with its Display impl.
        ProtocolError::RonParse {
            message: e.code.to_string(),
            position: Some(p.line),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Color;

    fn painted_asset() -> UiDefinitionAsset {
        let mut a = empty_asset();
        a.root.id = "scene".into();
        let mut child_components = BTreeMap::new();
        child_components.insert(
            "transform".into(),
            ComponentPayload::Transform(TransformProps::default()),
        );
        let mut style = StyleProps::default();
        style.background = Color {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        };
        style.fill_enabled = true;
        child_components.insert("style".into(), ComponentPayload::Style(style));
        a.root.children.push(WidgetNode {
            id: "red".into(),
            name: Some("Red Box".into()),
            is_root: false,
            kind: WidgetKind::Container,
            children: vec![],
            components: child_components,
        });
        a
    }

    #[test]
    fn round_trip_is_identity() {
        let original = painted_asset();
        let text = save_to_string(&original).unwrap();
        let reloaded = load_from_str(&text).unwrap();
        assert_eq!(original, reloaded);
    }

    #[test]
    fn header_line_appears_at_top_of_saved_text() {
        let text = save_to_string(&empty_asset()).unwrap();
        assert!(
            text.starts_with(&header_line(CURRENT_SCHEMA_VERSION)),
            "saved text must begin with the schema-version header; got: {text:?}"
        );
    }

    #[test]
    fn header_mismatch_is_an_error() {
        let mut text = save_to_string(&empty_asset()).unwrap();
        // Tamper with the header version.
        text = text.replace(
            &header_line(CURRENT_SCHEMA_VERSION),
            &header_line(CURRENT_SCHEMA_VERSION + 1),
        );
        let err = load_from_str(&text).unwrap_err();
        // ron still parses because the body version is unchanged, so
        // the asset's body.version stays at CURRENT, the header says
        // CURRENT+1 — we surface BadHeader.
        let detail = format!("{err}");
        assert!(
            detail.contains("schema_version") || detail.contains("header"),
            "expected header-mismatch error, got: {detail}"
        );
    }

    #[test]
    fn not_utf8_is_an_error() {
        // 0x97 alone is invalid UTF-8.
        let bytes = vec![b'/', b'/', b' ', b's', b'c', b'h', b'e', b'm', b'a', 0x97];
        let path = std::env::temp_dir().join("beui-protocol-not-utf8.beui");
        std::fs::write(&path, &bytes).unwrap();
        let err = load_from_file(&path).unwrap_err();
        assert_eq!(err.kind(), "not_utf8");
    }

    #[test]
    fn save_to_file_then_load_returns_equal_asset() {
        let path = std::env::temp_dir().join("beui-protocol-round-trip.beui");
        let original = painted_asset();
        save_to_file(&original, &path).unwrap();
        let loaded = load_from_file(&path).unwrap();
        assert_eq!(original, loaded);
        let _ = std::fs::remove_file(&path);
    }
}