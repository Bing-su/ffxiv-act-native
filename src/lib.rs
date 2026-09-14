//! Generate a small ACT managed shim for a Rust `cdylib`.
//!
//! The generator reads `Advanced Combat Tracker.exe` and the public
//! `FFXIV_ACT_Plugin.Common.dll` from the official SDK.  It deliberately never
//! reads `FFXIV_ACT_Plugin.dll`.

#![cfg_attr(not(test), deny(clippy::unwrap_used))]

mod abi;
mod api;
mod error;
mod generate;
mod metadata;

pub use abi::{ABI_VERSION, RawClientApiV1, RawEventV1, RawHostApiV1, Status};
pub use api::{
    Combatant, Event, EventKind, NetworkBuff, Player, Plugin, PluginError, PluginInit,
    PluginResult, Repository, SubscriptionSet, UiCommand, UiControl, UiEvent,
};
pub use bytes::Bytes;
pub use error::{DecodeError, GenerateError};
#[cfg(feature = "embedded-contracts")]
pub use generate::build_shim;
pub use generate::{PluginConfig, generate};
pub use metadata::PluginMetadata;

/// Export the single entry point expected by a generated managed shim.
///
/// The consuming crate must be built as a `cdylib`.
#[macro_export]
macro_rules! export_plugin {
    ($plugin:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "system" fn ffxiv_act_native_entry_v1(
            host: *const $crate::RawHostApiV1,
            client: *mut $crate::RawClientApiV1,
        ) -> $crate::Status {
            // SAFETY: the managed shim owns both pointers and follows ABI v1.
            unsafe { $crate::__private::start::<$plugin>(host, client) }
        }
    };
}

#[doc(hidden)]
pub mod __private {
    pub use crate::abi::start;
}
