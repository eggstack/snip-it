//! Utility modules for snp.
//!
//! This module provides helper functions for:
//! - [`config`]: Configuration directory paths
//! - [`variables`]: Variable expansion and parsing
//! - [`toml_helpers`]: TOML escape sequence handling
//! - [`shell_keywords`]: Shell keyword expansion

pub mod config;
pub mod shell_keywords;
pub mod toml_helpers;
pub mod variables;

pub use variables::{
    expand_command, extract_variables_for_display, has_unmatched_angle_bracket, parse_variables,
    strip_escape_sequences,
};
