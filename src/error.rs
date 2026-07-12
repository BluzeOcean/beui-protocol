//! Error type for the protocol crate.
//!
//! Every public API returns `Result<T, ProtocolError>`. The variants are
//! structured so callers can pattern-match on the failure mode — "the
//! file is from a newer editor" is a different problem from "the file
//! has two roots" or "the file is not valid UTF-8".
//!
//! When you add a new failure mode, add a new variant here AND a new
//! `pub fn` constructor that takes the relevant context. Don't smuggle
//! context into the Display message and expect callers to string-match.

use std::path::PathBuf;
use thiserror::Error;

/// All failure modes a `.beui` producer or consumer can encounter.
///
/// The error is `non_exhaustive` so adding a new variant in a future
/// minor release is not a breaking change. Consumers should always
/// include a wildcard arm.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ProtocolError {
    /// The on-disk file is not valid UTF-8. Common cause: a stray
    /// CP1252 byte (e.g. an em-dash `0x97`) from a copy-paste.
    #[error("file is not valid UTF-8: {path}")]
    NotUtf8 {
        /// Path of the file that failed to load.
        path: PathBuf,
    },

    /// RON could not parse the file. The location info comes from the
    /// underlying `ron` error and points at the byte offset that broke.
    #[error("RON parse error: {message} (offset {position:?})")]
    RonParse {
        /// Human-readable parse error from `ron`.
        message: String,
        /// Byte offset in the source where parsing failed.
        position: Option<usize>,
    },

    /// The file declares a `version` newer than this crate supports.
    /// Callers should refuse and tell the user to upgrade, NOT silently
    /// load fields they may not understand.
    #[error(
        "asset version {file_version} is newer than supported {supported} — upgrade the editor"
    )]
    AssetTooNew {
        /// The version number found in the file.
        file_version: u32,
        /// The current `CURRENT_SCHEMA_VERSION` this crate supports.
        supported: u32,
    },

    /// The file declares a `version` older than `MIN_SUPPORTED_VERSION`
    /// and there is no migration path forward.
    #[error(
        "asset version {file_version} is older than the minimum supported {min_supported} (current {supported}); no migration path"
    )]
    AssetTooOld {
        /// The version number found in the file.
        file_version: u32,
        /// The oldest version this crate can migrate forward.
        min_supported: u32,
        /// The current `CURRENT_SCHEMA_VERSION`.
        supported: u32,
    },

    /// The asset's version is recognized but the migration failed
    /// (data shape incompatible, required field missing, etc.).
    #[error("migration from v{from} to v{to} failed: {reason}")]
    MigrationFailed {
        /// Version being migrated from.
        from: u32,
        /// Version being migrated to.
        to: u32,
        /// Why the migration failed.
        reason: String,
    },

    /// The asset's required header `// schema_version: N` is missing or
    /// mismatches the `version:` field in the RON body. Some hand-written
    /// files omit the comment; the protocol requires it.
    #[error(
        "schema_version header missing or mismatched: header={header:?} body={body}"
    )]
    BadHeader {
        /// Version parsed from the `// schema_version:` comment, if any.
        header: Option<u32>,
        /// Version parsed from the `version:` field in the RON body.
        body: u32,
    },

    /// An invariant the protocol guarantees was violated. Examples:
    /// two widgets sharing an id within the same tree, more than one
    /// widget with `is_root: true`, an `Image` widget without an `image`
    /// component, a `Container` with no `transform` component after
    /// migration, etc.
    #[error("invariant violated: {kind} — {detail}")]
    InvariantViolation {
        /// Short identifier for the invariant, e.g. `"duplicate_id"` or
        /// `"multiple_roots"`. Programmatic consumers can match on this.
        kind: &'static str,
        /// Human-readable description including the offending widget id.
        detail: String,
    },

    /// I/O failure when reading or writing a file. Wraps the underlying
    /// `std::io::Error`.
    #[error("io error at {path}: {source}")]
    Io {
        /// Path that failed.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
}

impl ProtocolError {
    /// Programmatic classification of an error. Used by tests + agent
    /// tooling that needs to handle failure modes without parsing the
    /// Display message.
    pub fn kind(&self) -> &'static str {
        match self {
            ProtocolError::NotUtf8 { .. } => "not_utf8",
            ProtocolError::RonParse { .. } => "ron_parse",
            ProtocolError::AssetTooNew { .. } => "asset_too_new",
            ProtocolError::AssetTooOld { .. } => "asset_too_old",
            ProtocolError::MigrationFailed { .. } => "migration_failed",
            ProtocolError::BadHeader { .. } => "bad_header",
            ProtocolError::InvariantViolation { .. } => "invariant_violation",
            ProtocolError::Io { .. } => "io",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_returns_stable_identifiers() {
        // Locked — agent tooling depends on these strings.
        assert_eq!(
            ProtocolError::NotUtf8 { path: PathBuf::from("x") }.kind(),
            "not_utf8"
        );
        assert_eq!(
            ProtocolError::AssetTooNew {
                file_version: 4,
                supported: 3
            }
            .kind(),
            "asset_too_new"
        );
        assert_eq!(
            ProtocolError::InvariantViolation {
                kind: "duplicate_id",
                detail: "x".into()
            }
            .kind(),
            "invariant_violation"
        );
    }
}