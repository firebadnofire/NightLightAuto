// SPDX-License-Identifier: MPL-2.0
/// Typed adapter failures retain the underlying library error until presentation.
#[derive(Debug, thiserror::Error)]
pub enum IntegrationError {
    #[error("Night Light {operation}: {source}")]
    NightLight {
        operation: &'static str,
        #[source]
        source: win_nightlight_lib::Error,
    },
    #[error("Night Light changed concurrently; cancelled the write to preserve external changes")]
    ConcurrentChange,
    #[error("Night Light write verification did not match the requested state")]
    Verification,
}
