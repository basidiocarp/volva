//! Hook adapter registry.
//!
//! This crate provides the hardcoded list of available hook adapters.
//! Currently static; future implementations may add filesystem discovery or manifest-based loading.

#[must_use]
pub fn adapter_names() -> Vec<&'static str> {
    vec!["hyphae", "rhizome", "cortina", "canopy", "stipe"]
}
