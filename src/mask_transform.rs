//! App side of `mask_transform`: the parts that need the window, the app state or
//! app-only modules. The engine part is `dudel_core::mask_transform`, re-exported
//! here so `crate::mask_transform::…` paths keep working.
pub(crate) use dudel_core::mask_transform::*;

#[cfg(test)]
#[path = "mask_transform_tests.rs"]
mod tests;
