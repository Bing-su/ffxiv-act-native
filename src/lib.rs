//! Generate a small ACT managed shim for a Rust `cdylib`.
//!
//! The generator reads `Advanced Combat Tracker.exe` and the public
//! `FFXIV_ACT_Plugin.Common.dll` from the official SDK.  It deliberately never
//! reads `FFXIV_ACT_Plugin.dll`.

#![cfg_attr(not(test), deny(clippy::unwrap_used))]

mod abi;
mod api;
mod generate;

pub use abi::{ABI_VERSION, RawClientApiV1, RawEventV1, RawHostApiV1, Status};
pub use api::{
    Combatant, Event, EventKind, NetworkBuff, Player, Plugin, PluginError, PluginResult,
    Repository, SubscriptionSet,
};
pub use bytes::Bytes;
pub use generate::{GenerateError, PluginConfig, generate};

/// Export the single entry point expected by a generated managed shim.
///
/// The consuming crate must be built as a `cdylib`.
#[macro_export]
macro_rules! export_plugin {
    ($plugin:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "system" fn act_bridge_entry_v1(
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
