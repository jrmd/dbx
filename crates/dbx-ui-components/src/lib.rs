//! Shared look-and-feel primitives: embedded assets, the theme catalog and
//! global theme state, accessibility helpers and reduced-motion detection.
//!
//! Keeping these in their own crate means edits to application code do not
//! recompile them, and examples can render real controls without including
//! the application binary.
pub mod accessibility;
pub mod assets;
pub mod motion;
pub mod theme;
