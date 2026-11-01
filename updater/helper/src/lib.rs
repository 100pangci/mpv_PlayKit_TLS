//! Transactional updater for the portable mpv PlayKit distribution.
//!
//! The binary is intentionally small and self-contained: it downloads a
//! complete release archive, verifies the release metadata and SHA-256 chain,
//! stages the new installation, re-applies the user's semantic patches, and
//! only then swaps directories.

pub mod archive;
pub mod cli;
pub mod config;
pub mod download;
pub mod manifest;
pub mod patch;
pub mod state;
pub mod status;
pub mod swap;
pub mod update;
pub mod util;
