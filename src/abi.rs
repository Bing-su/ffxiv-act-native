use std::{
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr::null_mut,
    slice,
    sync::{Mutex, OnceLock},
};

use crate::{Event, Plugin, PluginInit, Repository};

/// Version implemented by the raw managed/native ABI types in this crate.
///
/// Both sides check this value before exchanging callbacks so incompatible
/// structure layouts fail cleanly instead of causing undefined behavior.
pub const ABI_VERSION: u32 = 1;

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Status code returned across the managed/native ABI boundary.
///
/// A fixed integer representation lets C# consume failures without depending
/// on Rust-specific error layout.
pub enum Status {
    /// The operation completed successfully.
    Ok = 0,
    /// A pointer, payload, or other argument was invalid.
    InvalidArgument = 1,
    /// The caller and plugin use different ABI versions.
    AbiMismatch = 2,
    /// The plugin or requested host service is not initialized.
    NotInitialized = 3,
    /// Plugin code returned an error.
    PluginError = 4,
    /// A panic was caught before it could cross FFI.
    Panic = 5,
    /// The supplied output buffer is too small and should be retried.
    BufferTooSmall = 6,
    /// The operation cannot continue because shutdown has begun.
    Shutdown = 7,
}

/// Managed host callback used to execute a repository query.
///
/// The first call may use a null output pointer to discover `required`; a retry
/// supplies that many bytes. This two-call shape avoids transferring allocator
/// ownership across FFI.
///
/// # Safety
///
/// All non-null pointers must be valid for their stated lengths for the duration
/// of the call, and `required` must be writable.
pub type QueryFn = unsafe extern "system" fn(
    context: *mut c_void,
    query: u32,
    request: *const u8,
    request_len: usize,
    output: *mut u8,
    output_len: usize,
    required: *mut usize,
) -> Status;

/// Managed host callback used to display plugin status text in ACT.
///
/// # Safety
///
/// The byte pointer must contain valid UTF-8 for the supplied length and remain
/// readable for the duration of the call.
pub type SetStatusFn = unsafe extern "system" fn(*mut c_void, *const u8, usize);

#[repr(C)]
#[derive(Clone, Copy)]
/// Function table supplied by the managed shim to a native plugin.
///
/// `repr(C)` and the version field make this table safe to mirror in managed
/// interop declarations after the caller validates [`ABI_VERSION`].
pub struct RawHostApiV1 {
    /// ABI version used to lay out this table.
    pub abi_version: u32,
    /// Opaque value passed back to every host callback.
    pub context: *mut c_void,
    /// Optional repository-query callback.
    pub query: Option<QueryFn>,
    /// Optional callback for reporting plugin errors to ACT.
    pub set_status: Option<SetStatusFn>,
}

// SAFETY: managed delegates may enter on different CLR threads, but the opaque
// context is only passed back to host functions while one global lock serializes
// plugin callbacks; Rust code never dereferences it.
unsafe impl Send for RawHostApiV1 {}
unsafe impl Sync for RawHostApiV1 {}

#[repr(C)]
#[derive(Clone, Copy)]
/// Borrowed event descriptor passed from the managed shim.
///
/// The payload remains owned by the caller and is decoded only during the event
/// callback, avoiding a copy at the FFI boundary.
pub struct RawEventV1 {
    /// Numeric [`crate::EventKind`] tag.
    pub kind: u32,
    /// Start of the encoded payload, or null when `payload_len` is zero.
    pub payload: *const u8,
    /// Number of readable bytes at `payload`.
    pub payload_len: usize,
}

/// Native callback used by the managed shim to deliver one event.
///
/// # Safety
///
/// `RawEventV1` and its payload must remain readable for the duration of the call.
pub type EventFn = unsafe extern "system" fn(*mut c_void, *const RawEventV1) -> Status;
/// Native callback used by the managed shim to release the plugin.
///
/// # Safety
///
/// The context must be the value published in [`RawClientApiV1`].
pub type ShutdownFn = unsafe extern "system" fn(*mut c_void) -> Status;

#[repr(C)]
#[derive(Clone, Copy)]
/// Function table returned by the native plugin to the managed shim.
///
/// The table publishes only stable callbacks and primitive values so neither
/// runtime must understand the other's object layout.
pub struct RawClientApiV1 {
    /// ABI version used to lay out this table.
    pub abi_version: u32,
    /// Raw bits of the requested [`crate::SubscriptionSet`].
    pub subscriptions: u32,
    /// Opaque value passed to native callbacks.
    pub context: *mut c_void,
    /// Callback that accepts subscribed ACT events.
    pub on_event: EventFn,
    /// Callback that releases native plugin state.
    pub shutdown: ShutdownFn,
}

trait ErasedPlugin: Send {
    fn event(&mut self, host: &RawHostApiV1, raw: &RawEventV1) -> Status;
    fn shutdown(&mut self, host: &RawHostApiV1) -> Status;
}

struct Running<P>(P);

impl<P: Plugin> ErasedPlugin for Running<P> {
    fn event(&mut self, host: &RawHostApiV1, raw: &RawEventV1) -> Status {
        let bytes = if raw.payload_len == 0 {
            &[]
        } else if raw.payload.is_null() {
            return Status::InvalidArgument;
        } else {
            // SAFETY: the managed caller promises the payload lives for this call.
            unsafe { slice::from_raw_parts(raw.payload, raw.payload_len) }
        };
        let event = match Event::decode(raw.kind, bytes) {
            Ok(event) => event,
            Err(_) => return Status::InvalidArgument,
        };
        match self.0.on_event(Repository::new(host), event) {
            Ok(()) => Status::Ok,
            Err(error) => {
                report(host, &error.to_string());
                Status::PluginError
            }
        }
    }

    fn shutdown(&mut self, host: &RawHostApiV1) -> Status {
        match self.0.shutdown(Repository::new(host)) {
            Ok(()) => Status::Ok,
            Err(error) => {
                report(host, &error.to_string());
                Status::PluginError
            }
        }
    }
}

struct State {
    host: RawHostApiV1,
    plugin: Box<dyn ErasedPlugin>,
    failed: bool,
}

static STATE: OnceLock<Mutex<Option<State>>> = OnceLock::new();

fn state() -> &'static Mutex<Option<State>> {
    STATE.get_or_init(|| Mutex::new(None))
}

/// Starts `P` for the managed ABI v1 caller.
///
/// This is public only to support [`crate::export_plugin`]; plugin crates should
/// export the macro instead of calling this function directly.
///
/// # Safety
///
/// `host` and `client` must either both be null to request shutdown, or point
/// to readable/writable ABI v1 structures that remain valid for this call.
pub unsafe fn start<P: Plugin>(host: *const RawHostApiV1, client: *mut RawClientApiV1) -> Status {
    if host.is_null() && client.is_null() {
        return unsafe { shutdown(null_mut()) };
    }
    if host.is_null() || client.is_null() {
        return Status::InvalidArgument;
    }
    // SAFETY: validated non-null and ABI structs are copied immediately.
    let host = unsafe { *host };
    if host.abi_version != ABI_VERSION {
        return Status::AbiMismatch;
    }

    if state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .is_some()
    {
        return Status::InvalidArgument;
    }

    let result = catch_unwind(AssertUnwindSafe(|| P::init(Repository::new(&host))));
    let PluginInit {
        plugin,
        subscriptions,
    } = match result {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => {
            report(&host, &error.to_string());
            return Status::PluginError;
        }
        Err(_) => {
            report(&host, "FFXIV ACT native: plugin panicked during init");
            return Status::Panic;
        }
    };

    let mut guard = state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.is_some() {
        return Status::InvalidArgument;
    }
    *guard = Some(State {
        host,
        plugin: Box::new(Running(plugin)),
        failed: false,
    });
    // SAFETY: the caller supplied writable storage for ABI v1.
    unsafe {
        *client = RawClientApiV1 {
            abi_version: ABI_VERSION,
            subscriptions: subscriptions.bits(),
            context: null_mut(),
            on_event,
            shutdown,
        };
    }
    Status::Ok
}

unsafe extern "system" fn on_event(_: *mut c_void, raw: *const RawEventV1) -> Status {
    if raw.is_null() {
        return Status::InvalidArgument;
    }
    let mut guard = state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(state) = guard.as_mut() else {
        return Status::NotInitialized;
    };
    if state.failed {
        return Status::PluginError;
    }
    // SAFETY: pointer was checked and is only borrowed during this call.
    let result = catch_unwind(AssertUnwindSafe(|| {
        state.plugin.event(&state.host, unsafe { &*raw })
    }));
    let panicked = result.is_err();
    let status = result.unwrap_or(Status::Panic);
    if status != Status::Ok {
        state.failed = true;
        if panicked {
            report(
                &state.host,
                "FFXIV ACT native: plugin panicked during event callback",
            );
        }
    }
    status
}

unsafe extern "system" fn shutdown(_: *mut c_void) -> Status {
    catch_unwind(AssertUnwindSafe(|| {
        let mut guard = state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(mut state) = guard.take() else {
            return Status::NotInitialized;
        };
        state.plugin.shutdown(&state.host)
    }))
    .unwrap_or(Status::Panic)
}

fn report(host: &RawHostApiV1, message: &str) {
    let Some(set_status) = host.set_status else {
        return;
    };
    // SAFETY: message remains alive for the duration of the call.
    unsafe { set_status(host.context, message.as_ptr(), message.len()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PluginResult, SubscriptionSet};
    use std::{
        ptr::null,
        sync::atomic::{AtomicUsize, Ordering},
    };

    static EVENTS: AtomicUsize = AtomicUsize::new(0);
    static INITS: AtomicUsize = AtomicUsize::new(0);

    struct TestPlugin;
    impl Plugin for TestPlugin {
        fn init(_: Repository<'_>) -> PluginResult<PluginInit<Self>> {
            INITS.fetch_add(1, Ordering::SeqCst);
            Ok(PluginInit::new(
                Self,
                SubscriptionSet::PRIMARY_PLAYER_CHANGED,
            ))
        }
        fn on_event(&mut self, _: Repository<'_>, _: Event<'_>) -> PluginResult<()> {
            EVENTS.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn shutdown(&mut self, _: Repository<'_>) -> PluginResult<()> {
            Ok(())
        }
    }

    unsafe extern "system" fn query(
        _: *mut c_void,
        _: u32,
        _: *const u8,
        _: usize,
        _: *mut u8,
        _: usize,
        required: *mut usize,
    ) -> Status {
        if !required.is_null() {
            unsafe { *required = 0 }
        }
        Status::Ok
    }
    unsafe extern "system" fn status(_: *mut c_void, _: *const u8, _: usize) {}

    #[test]
    fn lifecycle_is_panic_safe() {
        let host = RawHostApiV1 {
            abi_version: ABI_VERSION,
            context: null_mut(),
            query: Some(query),
            set_status: Some(status),
        };
        let mut client = RawClientApiV1 {
            abi_version: 0,
            subscriptions: 0,
            context: null_mut(),
            on_event,
            shutdown,
        };
        assert_eq!(
            unsafe { start::<TestPlugin>(&host, &mut client) },
            Status::Ok
        );
        assert_eq!(
            unsafe { start::<TestPlugin>(&host, &mut client) },
            Status::InvalidArgument
        );
        assert_eq!(INITS.load(Ordering::SeqCst), 1);
        let raw = RawEventV1 {
            kind: 4,
            payload: null(),
            payload_len: 0,
        };
        assert_eq!(
            unsafe { (client.on_event)(client.context, &raw) },
            Status::Ok
        );
        assert_eq!(EVENTS.load(Ordering::SeqCst), 1);
        assert_eq!(unsafe { (client.shutdown)(client.context) }, Status::Ok);
    }
}
