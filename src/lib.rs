//! Generate a small ACT managed shim for a Rust `cdylib`.
//!
//! The generator reads `Advanced Combat Tracker.exe` and the public
//! `FFXIV_ACT_Plugin.Common.dll` from the official SDK.  It deliberately never
//! reads `FFXIV_ACT_Plugin.dll`.
//!
//! The split keeps plugin code native while limiting the generated managed
//! assembly to the small compatibility layer ACT requires.
//!
//! # Plugin skeleton
//!
//! ```no_run
//! use ffxiv_act_native::{
//!     Event, Plugin, PluginInit, PluginResult, Repository, SubscriptionSet,
//! };
//!
//! struct ExamplePlugin;
//!
//! impl Plugin for ExamplePlugin {
//!     fn init(_: Repository<'_>) -> PluginResult<PluginInit<Self>> {
//!         Ok(PluginInit::new(Self, SubscriptionSet::ZONE_CHANGED))
//!     }
//!
//!     fn on_event(&mut self, _: Repository<'_>, event: Event<'_>) -> PluginResult<()> {
//!         if let Event::ZoneChanged { id, name } = event {
//!             println!("entered {name} ({id})");
//!         }
//!         Ok(())
//!     }
//!
//!     fn shutdown(&mut self, _: Repository<'_>) -> PluginResult<()> {
//!         Ok(())
//!     }
//! }
//!
//! ffxiv_act_native::export_plugin!(ExamplePlugin);
//! ```

#![cfg_attr(not(test), deny(clippy::unwrap_used))]

mod abi;
mod api;
mod error;
mod generate;
mod metadata;

pub use abi::{ABI_VERSION, RawClientApiV1, RawEventV1, RawHostApiV1, Status};
pub use api::{
    Combatant, Event, EventKind, NetworkBuff, Player, Plugin, PluginError, PluginInit,
    PluginResult, Repository, RepositoryError, SubscriptionSet, UiCommand, UiControl, UiEvent,
};
pub use bytes::Bytes;
pub use error::{DecodeError, GenerateError};
#[cfg(feature = "embedded-contracts")]
pub use generate::build_shim;
pub use generate::{PluginConfig, generate};
pub use metadata::PluginMetadata;

/// Export the single entry point expected by a generated managed shim.
///
/// Use this once in the plugin crate so the managed shim has a stable ABI symbol
/// to load. The consuming crate must be built as a `cdylib`.
///
/// # Example
///
/// ```ignore
/// ffxiv_act_native::export_plugin!(MyPlugin);
/// ```
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
