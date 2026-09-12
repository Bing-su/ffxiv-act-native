use std::{
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    slice,
    sync::{Mutex, OnceLock},
};

use crate::{CallContext, Event, Plugin, Repository};

pub const ABI_VERSION: u32 = 1;

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Ok = 0,
    InvalidArgument = 1,
    AbiMismatch = 2,
    NotInitialized = 3,
    PluginError = 4,
    Panic = 5,
    BufferTooSmall = 6,
    Shutdown = 7,
}

pub type QueryFn = unsafe extern "system" fn(
    context: *mut c_void,
    query: u32,
    request: *const u8,
    request_len: usize,
    output: *mut u8,
    output_len: usize,
    required: *mut usize,
) -> Status;

pub type SetStatusFn = unsafe extern "system" fn(*mut c_void, *const u8, usize);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct RawHostApiV1 {
    pub abi_version: u32,
    pub context: *mut c_void,
    pub query: Option<QueryFn>,
    pub set_status: Option<SetStatusFn>,
}

// Managed delegates can enter on different CLR threads. The plugin is kept
// behind one lock so user code only observes serialized callbacks.
unsafe impl Send for RawHostApiV1 {}
unsafe impl Sync for RawHostApiV1 {}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct RawEventV1 {
    pub kind: u32,
    pub payload: *const u8,
    pub payload_len: usize,
}

pub type EventFn = unsafe extern "system" fn(*mut c_void, *const RawEventV1) -> Status;
pub type ShutdownFn = unsafe extern "system" fn(*mut c_void) -> Status;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct RawClientApiV1 {
    pub abi_version: u32,
    pub subscriptions: u32,
    pub context: *mut c_void,
    pub on_event: EventFn,
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
        let repository = Repository::new(host);
        let context = CallContext::new(repository);
        match self.0.on_event(context, event) {
            Ok(()) => Status::Ok,
            Err(error) => {
                report(host, &error.to_string());
                Status::PluginError
            }
        }
    }

    fn shutdown(&mut self, host: &RawHostApiV1) -> Status {
        let context = CallContext::new(Repository::new(host));
        match self.0.shutdown(context) {
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
/// # Safety
///
/// `host` and `client` must either both be null to request shutdown, or point
/// to readable/writable ABI v1 structures that remain valid for this call.
pub unsafe fn start<P: Plugin>(host: *const RawHostApiV1, client: *mut RawClientApiV1) -> Status {
    if host.is_null() && client.is_null() {
        return unsafe { shutdown(std::ptr::null_mut()) };
    }
    if host.is_null() || client.is_null() {
        return Status::InvalidArgument;
    }
    // SAFETY: validated non-null and ABI structs are copied immediately.
    let host = unsafe { *host };
    if host.abi_version != ABI_VERSION {
        return Status::AbiMismatch;
    }

    let result = catch_unwind(AssertUnwindSafe(|| {
        let context = CallContext::new(Repository::new(&host));
        P::init(context)
    }));
    let (plugin, subscriptions) = match result {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => {
            report(&host, &error.to_string());
            return Status::PluginError;
        }
        Err(_) => {
            report(&host, "Rust bridge: plugin panicked during init");
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
            context: std::ptr::null_mut(),
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
                "Rust bridge: plugin panicked during event callback",
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    static EVENTS: AtomicUsize = AtomicUsize::new(0);

    struct TestPlugin;
    impl Plugin for TestPlugin {
        fn init(_: CallContext<'_>) -> PluginResult<(Self, SubscriptionSet)> {
            Ok((Self, SubscriptionSet::ZONE_CHANGED))
        }
        fn on_event(&mut self, _: CallContext<'_>, _: Event<'_>) -> PluginResult<()> {
            EVENTS.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn shutdown(&mut self, _: CallContext<'_>) -> PluginResult<()> {
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
            context: std::ptr::null_mut(),
            query: Some(query),
            set_status: Some(status),
        };
        let mut client = RawClientApiV1 {
            abi_version: 0,
            subscriptions: 0,
            context: std::ptr::null_mut(),
            on_event,
            shutdown,
        };
        assert_eq!(
            unsafe { start::<TestPlugin>(&host, &mut client) },
            Status::Ok
        );
        let raw = RawEventV1 {
            kind: 6,
            payload: std::ptr::null(),
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
