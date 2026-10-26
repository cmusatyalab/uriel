//! C ABI. Every function returns a [`Status`]; handles are opaque pointers
//! created by `*_create` and freed by `*_destroy`, and a function that acts on a handle takes it
//! first. After a failure, [`uriel_last_error`] says what went wrong.
//!
//! The ABI has a version, [`ABI_VERSION`], which changes whenever a call or a struct changes in a
//! way that breaks code built against the previous version. A binding that loads the library at
//! run time compares the header's version with [`uriel_abi_version`]. Enums are 32 bits wide,
//! and new variants may be added at the end, so code should handle values it does not know.
//!
//! A producer, an engine, a subscription and a reader must stay on the thread that created them:
//! create, use and destroy each on one thread. A client and a server may be used from any thread.

use std::{
    cell::RefCell,
    ffi::{CStr, CString, c_char},
    path::PathBuf,
    ptr,
    time::Duration,
};

use iceoryx2_bb_container::string::String as _;

use crate::{
    Client, Engine, Error, EventKind, FlowSpec, InputId, Outcome, Producer, Reader,
    RemoteServerConfig, Result, Server, Subscription, logging, reader::ReadInput,
    routing::ids::ProducerName, subscription::Event,
};

/// The version of the ABI that this library implements.
pub const ABI_VERSION: u32 = 3;

/// The version of the ABI that the library implements. A caller built with a header of another
/// version (`URIEL_ABI_VERSION`) must not use the library.
#[unsafe(no_mangle)]
pub extern "C" fn uriel_abi_version() -> u32 {
    ABI_VERSION
}

/// Status code returned by every C ABI function.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The call succeeded.
    Ok = 0,
    /// An argument was invalid, or the call is not allowed in the object's state.
    ErrInvalid,
    /// Data was larger than the limit it was created with.
    ErrSizeExceeded,
    /// Nothing is ready yet: no input or event to poll, or a wait timed out. Try again.
    ErrAgain,
    /// Something failed inside Uriel, e.g. shared memory or the network.
    ErrInternal,
}

thread_local! {
    /// The message of the last failure on this thread.
    static LAST_ERROR: RefCell<CString> = RefCell::default();
}

/// Records `err` as this thread's last error, for [`uriel_last_error`].
fn set_last_error(err: &Error) {
    // A message with a NUL in it is cut there, rather than lost.
    let message = err.to_string();
    let message = CString::new(message).unwrap_or_else(|e| {
        let nul = e.nul_position();
        let mut bytes = e.into_vec();
        bytes.truncate(nul);
        CString::new(bytes).expect("cut before the first NUL")
    });
    LAST_ERROR.with(|last| *last.borrow_mut() = message);
}

/// Says what went wrong in the last call on this thread that failed. The string stays valid until
/// the next call on this thread that fails. It is "" if no call has failed yet.
#[unsafe(no_mangle)]
pub extern "C" fn uriel_last_error() -> *const c_char {
    LAST_ERROR.with(|last| last.borrow().as_ptr())
}

impl From<&Error> for Status {
    fn from(err: &Error) -> Self {
        set_last_error(err);
        match err {
            Error::InvalidArgument(_) => Status::ErrInvalid,
            Error::SizeExceeded { .. } => Status::ErrSizeExceeded,
            Error::Again => Status::ErrAgain,
            Error::Internal(_) => Status::ErrInternal,
        }
    }
}

impl From<Result<()>> for Status {
    fn from(result: Result<()>) -> Self {
        match result {
            Ok(()) => Status::Ok,
            Err(err) => Status::from(&err),
        }
    }
}

/// Borrows a C string as `&str`.
///
/// # Safety
/// `ptr` must be null or point to a NUL-terminated string that outlives `'a`.
unsafe fn str_arg<'a>(ptr: *const c_char) -> Result<&'a str> {
    if ptr.is_null() {
        return Err(Error::InvalidArgument("null string".into()));
    }
    // SAFETY: non-null and NUL-terminated per the caller's contract.
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map_err(|e| Error::InvalidArgument(format!("string is not valid UTF-8: {e}")))
}

/// Like [`str_arg`], but null means no string.
///
/// # Safety
/// As for [`str_arg`].
unsafe fn opt_str_arg<'a>(ptr: *const c_char) -> Result<Option<&'a str>> {
    if ptr.is_null() {
        return Ok(None);
    }
    // SAFETY: non-null, and valid per the caller's contract.
    unsafe { str_arg(ptr) }.map(Some)
}

/// Borrows an opaque handle mutably.
///
/// # Safety
/// `ptr` must be null or a live handle returned by the matching `*_create`.
unsafe fn handle_arg<'a, T>(ptr: *mut T) -> Result<&'a mut T> {
    // SAFETY: valid or null per the caller's contract.
    unsafe { ptr.as_mut() }.ok_or_else(|| Error::InvalidArgument("null handle".into()))
}

/// Stores a newly created object in `*out` as an owned, opaque handle.
///
/// # Safety
/// `out` must be null or valid for writes.
unsafe fn create_handle<T>(out: *mut *mut T, create: impl FnOnce() -> Result<T>) -> Status {
    if out.is_null() {
        return null_argument("output pointer");
    }
    let result = create().map(|value| {
        // SAFETY: `out` is non-null and writable per the caller's contract.
        unsafe { *out = Box::into_raw(Box::new(value)) };
    });
    Status::from(result)
}

/// Fails a call that was given a null `name`.
fn null_argument(name: &str) -> Status {
    Status::from(&Error::InvalidArgument(format!("null {name}")))
}

/// Frees a handle returned by the matching `*_create`. Null is a no-op.
///
/// # Safety
/// `ptr` must be null or a live handle that is not used afterwards.
unsafe fn destroy_handle<T>(ptr: *mut T) -> Status {
    if !ptr.is_null() {
        // SAFETY: `ptr` came from `Box::into_raw` in `create_handle`.
        drop(unsafe { Box::from_raw(ptr) });
    }
    Status::Ok
}

// --- Logging ---

/// Prints Uriel's log to standard error. `filter` picks what is printed, in the syntax of
/// `RUST_LOG`, e.g. "info" or "uriel=debug". If it is NULL, `RUST_LOG` picks, or warnings and
/// errors are printed if that is unset. Call it once, before anything else; without it, nothing is
/// logged. Fails with `ERR_INVALID` for a malformed filter, or when called a second time.
///
/// # Safety
/// `filter` must be NUL-terminated or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_log_init(filter: *const c_char) -> Status {
    Status::from(unsafe { opt_str_arg(filter) }.and_then(logging::init))
}

// --- Metrics ---

/// Serves the metrics of Uriel's clients and servers in this process to Prometheus, at
/// `http://<addr>/metrics`, e.g. "0.0.0.0:9464". Call it once, before creating clients and servers.
/// Fails with `ERR_INVALID` for a malformed address. Also fails if the address is taken, or when
/// called a second time.
///
/// # Safety
/// `addr` must be NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_metrics_serve(addr: *const c_char) -> Status {
    let result = unsafe { str_arg(addr) }.and_then(|addr| {
        let addr = addr.parse().map_err(|e| {
            Error::InvalidArgument(format!("invalid metrics address {addr:?}: {e}"))
        })?;
        crate::metrics::serve(addr)
    });
    Status::from(result)
}

// --- Producer ---

/// Creates the producer `name`, which publishes the app's inputs.
///
/// - `name`: unique among the producers on this machine. A client takes the producer's inputs by
///   this name (`uriel_client_add_producer`), a flow spec lists it under `[[producers]]`, and
///   subscriptions and readers find it by it.
/// - `input_type`: how to read the inputs, e.g. a protobuf message name. Engines see it as each
///   input's `payload_type`. NULL if the inputs have no such convention.
/// - `max_input_size`: the size in bytes of the buffer that `uriel_producer_new_input` loans, and
///   so the largest input.
///
/// Use and destroy the producer only on the thread that created it.
///
/// # Safety
/// String arguments must be NUL-terminated (`input_type` may be null); `producer` must be valid
/// for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_producer_create(
    name: *const c_char,
    input_type: *const c_char,
    max_input_size: usize,
    producer: *mut *mut Producer,
) -> Status {
    unsafe {
        create_handle(producer, || {
            Producer::new(str_arg(name)?, opt_str_arg(input_type)?, max_input_size)
        })
    }
}

/// Destroys the producer. NULL is ignored.
///
/// # Safety
/// `producer` must be null or a live handle that is not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_producer_destroy(producer: *mut Producer) -> Status {
    unsafe { destroy_handle(producer) }
}

/// Loans a shared memory buffer of `max_input_size` bytes for the next input, and points `*buf`
/// to it. Write the input there, then call `uriel_producer_publish_input`. Calling it again before
/// publishing gives up the previous buffer.
///
/// # Safety
/// `producer` must be a live handle; `buf` must be valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_producer_new_input(
    producer: *mut Producer,
    buf: *mut *mut u8,
) -> Status {
    if buf.is_null() {
        return null_argument("buf");
    }
    let result = unsafe { handle_arg(producer) }
        .and_then(Producer::new_input)
        .map(|input| unsafe { *buf = input.as_mut_ptr().cast() });
    Status::from(result)
}

/// Publishes the first `len` bytes of the loaned buffer. The client that takes the producer's
/// inputs sends it through every flow that lists the producer (`uriel_client_add_flow`); if there
/// is none, nothing takes the input, and no event reports it. If `seq` is not null, it gets the
/// input's sequence number: 1 for the producer's first input, then one more for each. Events about
/// the input carry it. Fails with `ERR_SIZE_EXCEEDED` if `len` is more than `max_input_size`.
///
/// # Safety
/// `producer` must be a live handle; `seq` must be null or valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_producer_publish_input(
    producer: *mut Producer,
    len: usize,
    seq: *mut u64,
) -> Status {
    let result = unsafe { handle_arg(producer) }.and_then(|producer| producer.publish_input(len));
    Status::from(result.map(|id| {
        if !seq.is_null() {
            unsafe { seq.write(id.seq()) };
        }
    }))
}

/// Writes the producer's session to `*session`: a random number chosen when the producer was
/// created. Input ids carry it with the sequence number, so ids stay unique when a producer of the
/// same name restarts and counts from 1 again.
///
/// # Safety
/// `producer` must be a live handle; `session` must be valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_producer_session(
    producer: *mut Producer,
    session: *mut u64,
) -> Status {
    if session.is_null() {
        return null_argument("session");
    }
    let result = unsafe { handle_arg(producer) }.map(|producer| producer.session());
    Status::from(result.map(|value| unsafe { session.write(value) }))
}

// --- Client ---

/// How to create a client, beyond its name and local server. New options are added as new
/// setters, so `uriel_client_create` keeps its signature.
#[derive(Debug, Default)]
pub struct ClientOptions {
    remote: Option<RemoteServerConfig>,
    remote_crossing_capacity: Option<usize>,
}

/// Creates client options with the defaults: no remote server.
///
/// # Safety
/// `options` must be valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_client_options_create(options: *mut *mut ClientOptions) -> Status {
    unsafe { create_handle(options, || Ok(ClientOptions::default())) }
}

/// Destroys the options. NULL is ignored.
///
/// # Safety
/// `options` must be null or a live handle that is not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_client_options_destroy(options: *mut ClientOptions) -> Status {
    unsafe { destroy_handle(options) }
}

/// Makes the client also use a remote server, e.g. on a cloudlet, for the engines that a flow spec
/// marks `location = "remote"`.
///
/// - `addr`: the server's IP address and port, e.g. "192.0.2.1:4433" or "[2001:db8::1]:4433". A
///   host name is not accepted. The port is the one `uriel_server_remote_port` gives on the
///   server.
/// - `certificate`, `certificate_len`: the server's certificate in DER, as
///   `uriel_server_certificate` gives it on the server, e.g. copied from the `certificate.der` file
///   in its identity directory. The client trusts this certificate only.
///
/// The client connects on its own once it is created, and connects again after a loss; the
/// server may start later.
///
/// # Safety
/// `options` must be a live handle; `addr` must be NUL-terminated; `certificate` must point to
/// `certificate_len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_client_options_set_remote_server(
    options: *mut ClientOptions,
    addr: *const c_char,
    certificate: *const u8,
    certificate_len: usize,
) -> Status {
    let result = unsafe {
        (|| {
            let options = handle_arg(options)?;
            let addr = str_arg(addr)?;
            let addr = addr
                .parse()
                .map_err(|e| Error::InvalidArgument(format!("invalid address {addr:?}: {e}")))?;
            let certificate = bytes_arg(certificate, certificate_len)?.to_vec();
            options.remote = Some(RemoteServerConfig::new(addr, certificate));
            Ok(())
        })()
    };
    Status::from(result)
}

/// Sets how many inputs may be in flight, on the network or waiting at the server, on each edge of
/// a flow that crosses from this machine to the remote server. It must be at least 1, and needs a
/// remote server. The default, 3, keeps the remote engines on fresh inputs; a long round trip may
/// need more to keep them busy.
///
/// # Safety
/// `options` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_client_options_set_remote_crossing_capacity(
    options: *mut ClientOptions,
    capacity: usize,
) -> Status {
    let result = unsafe { handle_arg(options) }.map(|options| {
        options.remote_crossing_capacity = Some(capacity);
    });
    Status::from(result)
}

/// Creates the client `name`, which sends the inputs of its producers through its flows.
///
/// - `name`: unique among the clients on this machine.
/// - `local_server`: the name of the server on this machine that runs the flows' local engines,
///   as given to `uriel_server_create`, in this process or another one. It may start later.
/// - `options`: a remote server, and more; NULL for none. The client copies what it needs, so
///   they may be destroyed right after.
///
/// The client works on its own thread, and may be used from any thread.
///
/// # Safety
/// String arguments must be NUL-terminated; `options` must be null or a live handle; `client`
/// must be valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_client_create(
    name: *const c_char,
    local_server: *const c_char,
    options: *const ClientOptions,
    client: *mut *mut Client,
) -> Status {
    unsafe {
        create_handle(client, || {
            let options = options.as_ref();
            let mut remote = options.and_then(|options| options.remote.clone());
            if let Some(capacity) = options.and_then(|options| options.remote_crossing_capacity) {
                let remote = remote.as_mut().ok_or_else(|| {
                    Error::InvalidArgument(String::from(
                        "a remote crossing capacity needs a remote server",
                    ))
                })?;
                remote.crossing_capacity = capacity;
            }
            Client::new(str_arg(name)?, str_arg(local_server)?, remote)
        })
    }
}

/// Destroys the client, after removing its flows from the servers. NULL is ignored.
///
/// # Safety
/// `client` must be null or a live handle that is not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_client_destroy(client: *mut Client) -> Status {
    unsafe { destroy_handle(client) }
}

/// Starts taking inputs from the producer `name` on this machine, as given to
/// `uriel_producer_create`. The producer may be created before or after this call. A producer's
/// inputs go to one client only.
///
/// If `subscription` is not NULL, it gets a subscription to the events about the producer's
/// inputs, the same as `uriel_subscribe_producer`. It exists before the client takes the first
/// input, so it misses no event. It is created on the calling thread, and must be used and
/// destroyed there; to read the events on another thread, call this function on that thread. Pass
/// NULL if the app does not read the events.
///
/// # Safety
/// `client` must be a live handle; `name` must be NUL-terminated; `subscription` must be null or
/// valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_client_add_producer(
    client: *mut Client,
    name: *const c_char,
    subscription: *mut *mut Subscription,
) -> Status {
    let result = unsafe {
        (|| -> Result<()> {
            let client = handle_arg(client)?;
            let name = str_arg(name)?;
            let events = if subscription.is_null() {
                None
            } else {
                Some(Subscription::producer(name)?)
            };
            client.add_producer(name)?;
            if let Some(events) = events {
                // SAFETY: `subscription` is non-null and writable per the caller's contract.
                *subscription = Box::into_raw(Box::new(events));
            }
            Ok(())
        })()
    };
    Status::from(result)
}

/// Stops taking inputs from the producer `name`. Fails while a flow uses it.
///
/// # Safety
/// `client` must be a live handle; `name` must be NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_client_remove_producer(
    client: *mut Client,
    name: *const c_char,
) -> Status {
    Status::from(unsafe { (|| handle_arg(client)?.remove_producer(str_arg(name)?))() })
}

/// Adds the flow `name`: installs its engines on the servers, and routes the inputs that its
/// producers publish (`uriel_producer_publish_input`). A producer listed by several flows sends
/// each input through all of them.
///
/// - `name`: unique among this client's flows. Events about its inputs carry it.
/// - `spec`: the flow in TOML. It lists the producers, which the client must take already
///   (`uriel_client_add_producer`); the engines, by the names their servers serve them under,
///   each with `location = "local"` or `"remote"`; and the edges from a producer or an engine to
///   an engine, each with the time the engine has to answer:
///
/// ```toml
/// [[producers]]
/// name = "camera"
///
/// [[engines]]
/// name = "detector"
/// location = "local"
///
/// [[edges]]
/// from = "camera"
/// to = "detector"
/// timeout = "300ms"
/// ```
///
/// If the remote server is not connected yet, the remote part is installed once it is.
///
/// # Safety
/// `client` must be a live handle; string arguments must be NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_client_add_flow(
    client: *mut Client,
    name: *const c_char,
    spec: *const c_char,
) -> Status {
    Status::from(unsafe {
        (|| {
            let spec: FlowSpec = str_arg(spec)?.parse()?;
            handle_arg(client)?.add_flow(str_arg(name)?, &spec)
        })()
    })
}

/// Removes the flow `name` from the client and from the servers.
///
/// # Safety
/// `client` must be a live handle; `name` must be NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_client_remove_flow(
    client: *mut Client,
    name: *const c_char,
) -> Status {
    Status::from(unsafe { (|| handle_arg(client)?.remove_flow(str_arg(name)?))() })
}

// --- Server ---

/// How to create a server, beyond its name. New options are added as new setters, so
/// `uriel_server_create` keeps its signature.
#[derive(Debug, Default)]
pub struct ServerOptions {
    bind_addr: Option<String>,
    identity_dir: Option<PathBuf>,
}

/// Creates server options with the defaults: a server with local clients only.
///
/// # Safety
/// `options` must be valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_server_options_create(options: *mut *mut ServerOptions) -> Status {
    unsafe { create_handle(options, || Ok(ServerOptions::default())) }
}

/// Destroys the options. NULL is ignored.
///
/// # Safety
/// `options` must be null or a live handle that is not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_server_options_destroy(options: *mut ServerOptions) -> Status {
    unsafe { destroy_handle(options) }
}

/// Makes the server accept remote clients, over QUIC, on the local IP address and UDP port `addr`,
/// e.g. "0.0.0.0:4433" for every interface. Port 0 lets the system pick a free port, which
/// `uriel_server_remote_port` then gives. Without it, the server only serves clients on its
/// machine.
///
/// # Safety
/// `options` must be a live handle; `addr` must be NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_server_options_set_bind_addr(
    options: *mut ServerOptions,
    addr: *const c_char,
) -> Status {
    let result = unsafe {
        (|| {
            handle_arg(options)?.bind_addr = Some(str_arg(addr)?.to_owned());
            Ok(())
        })()
    };
    Status::from(result)
}

/// Makes a server that accepts remote clients keep its TLS certificate and key in `dir`, an
/// existing directory, as `certificate.der` and `key.der`. The server makes them there the first
/// time, and reuses them after, so clients keep trusting it across restarts; give clients
/// `certificate.der`. Without it, the server makes a new certificate each time it starts. It needs
/// a bind address.
///
/// # Safety
/// `options` must be a live handle; `dir` must be NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_server_options_set_identity_dir(
    options: *mut ServerOptions,
    dir: *const c_char,
) -> Status {
    let result = unsafe {
        (|| {
            handle_arg(options)?.identity_dir = Some(PathBuf::from(str_arg(dir)?));
            Ok(())
        })()
    };
    Status::from(result)
}

/// Creates the server `name`, which delivers the inputs of the flows installed on it to the engines
/// it serves on this machine.
///
/// - `name`: unique among the servers on this machine. Local clients find the server by it
///   (`local_server` in `uriel_client_create`).
/// - `options`: a bind address for remote clients, and more; NULL for a server with local clients
///   only. The server copies what it needs, so they may be destroyed right after.
///
/// The server works on its own thread, and may be used from any thread. `uriel-server` is a
/// ready-made program that runs one.
///
/// # Safety
/// `name` must be NUL-terminated; `options` must be null or a live handle; `server` must be valid
/// for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_server_create(
    name: *const c_char,
    options: *const ServerOptions,
    server: *mut *mut Server,
) -> Status {
    unsafe {
        create_handle(server, || {
            let options = options.as_ref();
            Server::new(
                str_arg(name)?,
                options.and_then(|options| options.bind_addr.as_deref()),
                options.and_then(|options| options.identity_dir.as_deref()),
            )
        })
    }
}

/// Destroys the server. NULL is ignored.
///
/// # Safety
/// `server` must be null or a live handle that is not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_server_destroy(server: *mut Server) -> Status {
    unsafe { destroy_handle(server) }
}

/// Writes the port remote clients reach the server on to `*port`. Fails for a server created
/// without a bind address.
///
/// # Safety
/// `server` must be a live handle; `port` must be valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_server_remote_port(server: *mut Server, port: *mut u16) -> Status {
    let result = unsafe {
        (|| {
            let addr = handle_arg(server)?.remote_addr().ok_or_else(not_remote)?;
            let port = port
                .as_mut()
                .ok_or_else(|| Error::InvalidArgument("null port".into()))?;
            *port = addr.port();
            Ok(())
        })()
    };
    Status::from(result)
}

/// Points `*certificate` to the certificate remote clients must trust, in DER, and writes its
/// length to `*len`. The bytes stay valid while the server lives. Fails for a server created
/// without a bind address.
///
/// # Safety
/// `server` must be a live handle; `certificate` and `len` must be valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_server_certificate(
    server: *mut Server,
    certificate: *mut *const u8,
    len: *mut usize,
) -> Status {
    let result = unsafe {
        (|| {
            let bytes = handle_arg(server)?.certificate().ok_or_else(not_remote)?;
            let (Some(certificate), Some(len)) = (certificate.as_mut(), len.as_mut()) else {
                return Err(Error::InvalidArgument("null output".into()));
            };
            *certificate = bytes.as_ptr();
            *len = bytes.len();
            Ok(())
        })()
    };
    Status::from(result)
}

fn not_remote() -> Error {
    Error::InvalidArgument("the server accepts no remote clients".into())
}

/// Starts serving the engine `name` on this machine, as given to `uriel_engine_create`, in this
/// process or another one. The engine may be created before or after this call. Flows name the
/// engine by the same name.
///
/// # Safety
/// `server` must be a live handle; `name` must be NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_server_add_engine(
    server: *mut Server,
    name: *const c_char,
) -> Status {
    Status::from(unsafe { (|| handle_arg(server)?.add_engine(str_arg(name)?))() })
}

/// Stops serving the engine `name`. Fails while an installed flow uses it.
///
/// # Safety
/// `server` must be a live handle; `name` must be NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_server_remove_engine(
    server: *mut Server,
    name: *const c_char,
) -> Status {
    Status::from(unsafe { (|| handle_arg(server)?.remove_engine(str_arg(name)?))() })
}

// --- Engine ---

/// Identifies one input of a producer. `producer` stays valid as long as the struct that holds
/// this id.
#[repr(C)]
pub struct InputIdView {
    /// The name of the producer that published the input.
    producer: *const c_char,
    /// The producer's session (`uriel_producer_session`).
    session: u64,
    /// The input's sequence number within the session, from 1.
    seq: u64,
}

/// An input handed to an engine. Stays valid until the engine reports its outcome.
#[repr(C)]
pub struct InputView {
    id: InputIdView,
    /// How to read `buf`; "" if the input has no convention.
    payload_type: *const c_char,
    buf: *const u8,
    len: usize,
}

/// Borrows `len` bytes at `ptr`; `ptr` may be null when `len` is 0.
///
/// # Safety
/// `ptr` must be null or point to `len` readable bytes that outlive `'a`.
unsafe fn bytes_arg<'a>(ptr: *const u8, len: usize) -> Result<&'a [u8]> {
    if len == 0 {
        return Ok(&[]);
    }
    if ptr.is_null() {
        return Err(Error::InvalidArgument("null buffer".into()));
    }
    // SAFETY: non-null and `len` readable bytes per the caller's contract.
    Ok(unsafe { std::slice::from_raw_parts(ptr, len) })
}

/// Creates the engine `name`, which processes inputs. It gets them once a server on this machine
/// serves it (`uriel_server_add_engine`) and a flow uses it.
///
/// - `name`: unique among the engines on this machine. Servers, flow specs and subscriptions name
///   the engine by it.
/// - `result_type`: how to read the results this engine reports, e.g. a protobuf message name.
///   Subscribers see it as each result's `payload_type`. NULL if they have no such convention.
/// - `max_output_size`: the size in bytes of the buffer that `uriel_engine_new_output` loans. A
///   result is at most this size, and so are a replacement input and its result together.
///
/// Use and destroy the engine only on the thread that created it.
///
/// # Safety
/// String arguments must be NUL-terminated (`result_type` may be null); `engine` must be valid
/// for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_engine_create(
    name: *const c_char,
    result_type: *const c_char,
    max_output_size: usize,
    engine: *mut *mut Engine,
) -> Status {
    unsafe {
        create_handle(engine, || {
            Engine::new(str_arg(name)?, opt_str_arg(result_type)?, max_output_size)
        })
    }
}

/// Destroys the engine. NULL is ignored.
///
/// # Safety
/// `engine` must be null or a live handle that is not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_engine_destroy(engine: *mut Engine) -> Status {
    unsafe { destroy_handle(engine) }
}

/// Waits until an input or an event of a subscription is ready, or `timeout_ms` milliseconds pass,
/// and returns `ERR_AGAIN` then. A timeout of 0 checks once without waiting. Fetch the input with
/// `uriel_engine_poll_input`, and the event with `uriel_engine_poll_event`; either may still
/// return `ERR_AGAIN`.
///
/// # Safety
/// `engine` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_engine_wait(engine: *mut Engine, timeout_ms: u32) -> Status {
    let timeout = Duration::from_millis(u64::from(timeout_ms));
    Status::from(unsafe { handle_arg(engine) }.and_then(|engine| engine.wait(timeout)))
}

/// Writes the next input to `*input`, or returns `ERR_AGAIN` if none is available. The input
/// stays valid until the engine reports its outcome, with `uriel_engine_drop`,
/// `uriel_engine_forward` or `uriel_engine_replace`; it must report one before polling the next.
///
/// # Safety
/// `engine` must be a live handle; `input` must be valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_engine_poll_input(
    engine: *mut Engine,
    input: *mut InputView,
) -> Status {
    if input.is_null() {
        return null_argument("input");
    }
    let result = unsafe { handle_arg(engine) }
        .and_then(Engine::poll_input)
        .map(|polled| {
            let view = InputView {
                id: InputIdView {
                    producer: polled.id.producer.as_c_str(),
                    session: polled.id.session,
                    seq: polled.id.seq,
                },
                payload_type: polled.payload_type.as_c_str(),
                buf: polled.data.as_ptr(),
                len: polled.data.len(),
            };
            // SAFETY: `input` is non-null and writable per the caller's contract.
            unsafe { input.write(view) };
        });
    Status::from(result)
}

/// Subscribes the engine to the events of the engine `name` on this machine, e.g. a detector whose
/// results it fuses: the same events as `uriel_subscribe_engine`. Only events published from now
/// on are read. `uriel_engine_wait` ends for them too.
///
/// # Safety
/// `engine` must be a live handle; `name` must be NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_engine_subscribe(
    engine: *mut Engine,
    name: *const c_char,
) -> Status {
    let result = unsafe { handle_arg(engine) }
        .and_then(|engine| engine.subscribe(unsafe { str_arg(name) }?));
    Status::from(result)
}

/// Writes the next event of the engine's subscriptions to `*event`, or returns `ERR_AGAIN` if none
/// is available. The event stays valid until the next poll. Its `engine` says which subscription
/// it came from.
///
/// # Safety
/// `engine` must be a live handle; `event` must be valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_engine_poll_event(
    engine: *mut Engine,
    event: *mut EventView,
) -> Status {
    if event.is_null() {
        return null_argument("event");
    }
    let result = unsafe { handle_arg(engine) }
        .and_then(Engine::poll_event)
        // SAFETY: `event` is non-null and writable per the caller's contract.
        .map(|polled| unsafe { event.write(EventView::new(&polled)) });
    Status::from(result)
}

/// Publishes `result`, which the engine derived from the events it subscribes to, about the input
/// `id` in the flow `flow`; e.g. a fusion decision. Pass the `id` and `flow` of the event the
/// result is about. `result_type` names how to read it, or is null. It answers no offer, so the
/// engine can publish it at any time. Its server publishes it as a `DERIVED` event on the engine's
/// event service. Fails with `ERR_SIZE_EXCEEDED` past the engine's maximum output size.
///
/// # Safety
/// `engine` must be a live handle; `id` must point to a valid input id; `flow` must be
/// NUL-terminated; `result_type` must be null or NUL-terminated; `result` must point to
/// `result_len` readable bytes, or be null when `result_len` is 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_engine_publish_derived(
    engine: *mut Engine,
    id: *const InputIdView,
    flow: *const c_char,
    result_type: *const c_char,
    result: *const u8,
    result_len: usize,
) -> Status {
    if id.is_null() {
        return null_argument("id");
    }
    let published = (|| {
        let engine = unsafe { handle_arg(engine) }?;
        // SAFETY: `id` is non-null and valid per the caller's contract.
        let id = unsafe { &*id };
        let producer = unsafe { str_arg(id.producer) }?;
        let id = InputId {
            producer: ProducerName::try_from(producer).map_err(|_| {
                Error::InvalidArgument(format!("producer name {producer:?} is too long"))
            })?,
            session: id.session,
            seq: id.seq,
        };
        let flow = unsafe { str_arg(flow) }?;
        let result_type = unsafe { opt_str_arg(result_type) }?.unwrap_or_default();
        let result = unsafe { bytes_arg(result, result_len) }?;
        engine.publish_derived(&id, flow, result_type, result)
    })();
    Status::from(published)
}

/// Reports the outcome drop: the engine forwards nothing, and the input ends there. `result`, if
/// any, reaches subscribers in the engine's `RESULT` event. A discard is a drop with no result
/// (`result` null, `result_len` 0). Fails with `ERR_SIZE_EXCEEDED` if `result_len` is more than
/// `max_output_size`.
///
/// # Safety
/// `engine` must be a live handle; `result` must point to `result_len` readable bytes, or be
/// null when `result_len` is 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_engine_drop(
    engine: *mut Engine,
    result: *const u8,
    result_len: usize,
) -> Status {
    Status::from(unsafe { (|| handle_arg(engine)?.drop(bytes_arg(result, result_len)?))() })
}

/// Reports the outcome forward: the engine forwards the input unchanged along every outgoing input
/// edge. `result` is as for `uriel_engine_drop`.
///
/// # Safety
/// As for `uriel_engine_drop`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_engine_forward(
    engine: *mut Engine,
    result: *const u8,
    result_len: usize,
) -> Status {
    Status::from(unsafe { (|| handle_arg(engine)?.forward(bytes_arg(result, result_len)?))() })
}

/// Loans a shared memory buffer of `max_output_size` bytes for a new input, and points `*buf` to
/// it. Fill it, then call `uriel_engine_replace`.
///
/// # Safety
/// `engine` must be a live handle; `buf` must be valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_engine_new_output(engine: *mut Engine, buf: *mut *mut u8) -> Status {
    if buf.is_null() {
        return null_argument("buf");
    }
    let result = unsafe { handle_arg(engine) }
        .and_then(Engine::new_output)
        // SAFETY: `buf` is non-null and writable per the caller's contract.
        .map(|output| unsafe { *buf = output.as_mut_ptr().cast() });
    Status::from(result)
}

/// Reports the outcome replace: the engine forwards the first `output_len` bytes of the buffer
/// that `uriel_engine_new_output` loaned, as a new input in place of the input, along every
/// outgoing input edge. `output_type` names how to read it; it may be NULL. `result` is as for
/// `uriel_engine_drop`. Fails with `ERR_SIZE_EXCEEDED` if `output_len` and `result_len` together
/// are more than `max_output_size`.
///
/// # Safety
/// As for `uriel_engine_drop`; `output_type` must be NUL-terminated or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_engine_replace(
    engine: *mut Engine,
    output_type: *const c_char,
    output_len: usize,
    result: *const u8,
    result_len: usize,
) -> Status {
    Status::from(unsafe {
        (|| {
            handle_arg(engine)?.replace(
                opt_str_arg(output_type)?.unwrap_or_default(),
                output_len,
                bytes_arg(result, result_len)?,
            )
        })()
    })
}

// --- Subscription ---

/// What happened, as a subscription reports it: the kinds of published events, and `Lost`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKindView {
    /// A server received the input, and made one offer of it per engine that takes it.
    Accepted,
    /// The offer was sent to the engine; a result may come.
    Sent,
    /// The engine reported its outcome for the input; `outcome` says which.
    Result,
    /// A newer input replaced the offer before it was sent, so the engine never saw it.
    Skipped,
    /// The engine did not answer in time, or was lost.
    TimedOut,
    /// The client lost its connection to a server of the flow `flow`. Every offer on that server
    /// ended with it. It names no input.
    Disconnected,
    /// A result an engine derived from the events it subscribes to
    /// (`uriel_engine_publish_derived`). It answers no offer.
    Derived,
    /// Events were lost before the next one, because the subscriber fell behind.
    Lost,
}

/// What an engine did with an input, as a subscription reports it.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeView {
    /// The engine forwarded nothing.
    Drop,
    /// The engine forwarded the input unchanged.
    Forward,
    /// The engine forwarded a new input in place of the input.
    Replace,
}

impl From<Outcome> for OutcomeView {
    fn from(outcome: Outcome) -> Self {
        match outcome {
            Outcome::Drop => Self::Drop,
            Outcome::Forward => Self::Forward,
            Outcome::Replace => Self::Replace,
        }
    }
}

impl From<EventKind> for EventKindView {
    fn from(kind: EventKind) -> Self {
        match kind {
            EventKind::Accepted => Self::Accepted,
            EventKind::Sent => Self::Sent,
            EventKind::Result => Self::Result,
            EventKind::Skipped => Self::Skipped,
            EventKind::TimedOut => Self::TimedOut,
            EventKind::Disconnected => Self::Disconnected,
            EventKind::Derived => Self::Derived,
        }
    }
}

/// An event read from a subscription. Stays valid until the next poll. For `LOST`, only `lost`
/// is set, and `engine` on a subscription to an engine; for `DISCONNECTED`, only `flow`, and
/// `engine` on a subscription to an engine.
#[repr(C)]
pub struct EventView {
    kind: EventKindView,
    /// For `LOST`: how many events were lost.
    lost: u64,
    id: InputIdView,
    /// The engine the event is about; "" if none.
    engine: *const c_char,
    /// The flow the event belongs to; "" if none.
    flow: *const c_char,
    /// What the engine did with the input, for `RESULT`.
    outcome: OutcomeView,
    /// How to read `result`; "" if there is no result.
    payload_type: *const c_char,
    result: *const u8,
    result_len: usize,
    /// When the event happened, in nanoseconds on the machine's `CLOCK_MONOTONIC`.
    at_ns: u64,
    /// When the server made the engine's offer, on the same clock; 0 if the client made the
    /// offer, or for `DISCONNECTED`.
    offered_at_ns: u64,
    /// When the client received the event from the remote server, on this machine's
    /// `CLOCK_MONOTONIC`; `at_ns` and `offered_at_ns` are then on the remote server's clock. 0 for
    /// other events.
    received_at_ns: u64,
}

impl EventView {
    fn new(event: &Event<'_>) -> Self {
        match event {
            Event::Lost(lost) => Self {
                kind: EventKindView::Lost,
                lost: lost.count(),
                id: InputIdView {
                    producer: c"".as_ptr(),
                    session: 0,
                    seq: 0,
                },
                engine: lost.engine.map_or(c"".as_ptr(), |engine| engine.as_c_str()),
                flow: c"".as_ptr(),
                outcome: OutcomeView::Drop,
                payload_type: c"".as_ptr(),
                result: ptr::null(),
                result_len: 0,
                at_ns: 0,
                offered_at_ns: 0,
                received_at_ns: 0,
            },
            Event::Published(published) => {
                let record = published.record;
                Self {
                    kind: record.kind.into(),
                    lost: 0,
                    id: InputIdView {
                        producer: record.id.producer.as_c_str(),
                        session: record.id.session,
                        seq: record.id.seq,
                    },
                    engine: record.engine.as_c_str(),
                    flow: record.flow.as_c_str(),
                    outcome: record.outcome.into(),
                    payload_type: record.payload_type.as_c_str(),
                    result: published.result().as_ptr(),
                    result_len: published.result().len(),
                    at_ns: published.at().as_nanos(),
                    offered_at_ns: published.offered_at().map_or(0, |at| at.as_nanos()),
                    received_at_ns: published.received_at().map_or(0, |at| at.as_nanos()),
                }
            }
        }
    }
}

/// Subscribes to the events about the inputs of the producer `name` on this machine: what happened
/// to each at each engine of its flows, local or remote, and lost connections. This is how an app
/// reads its results. The client that takes the producer's inputs publishes these events. Only
/// events published from now on are read. Use and destroy the subscription only on the thread
/// that created it.
///
/// # Safety
/// `name` must be NUL-terminated; `subscription` must be valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_subscribe_producer(
    name: *const c_char,
    subscription: *mut *mut Subscription,
) -> Status {
    unsafe { create_handle(subscription, || Subscription::producer(str_arg(name)?)) }
}

/// Subscribes to the events of the engine `name`: what happened to each input at that engine, from
/// every flow and producer. The engine's server publishes them, or for a remote engine, the client
/// on this machine that uses it. Only events published from now on are read. Use and destroy the
/// subscription only on the thread that created it.
///
/// # Safety
/// `name` must be NUL-terminated; `subscription` must be valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_subscribe_engine(
    name: *const c_char,
    subscription: *mut *mut Subscription,
) -> Status {
    unsafe { create_handle(subscription, || Subscription::engine(str_arg(name)?)) }
}

/// Destroys the subscription. NULL is ignored.
///
/// # Safety
/// `subscription` must be null or a live handle that is not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_subscription_destroy(subscription: *mut Subscription) -> Status {
    unsafe { destroy_handle(subscription) }
}

/// Waits until an event is ready, or `timeout_ms` milliseconds pass, and returns `ERR_AGAIN` then.
/// A timeout of 0 checks once without waiting. Fetch the event with `uriel_subscription_poll`.
///
/// # Safety
/// `subscription` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_subscription_wait(
    subscription: *mut Subscription,
    timeout_ms: u32,
) -> Status {
    let timeout = Duration::from_millis(u64::from(timeout_ms));
    Status::from(unsafe { handle_arg(subscription) }.and_then(|s| s.wait(timeout)))
}

/// Writes the next event to `*event`, or returns `ERR_AGAIN` if none is available. The event stays
/// valid until the next poll.
///
/// # Safety
/// `subscription` must be a live handle; `event` must be valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_subscription_poll(
    subscription: *mut Subscription,
    event: *mut EventView,
) -> Status {
    if event.is_null() {
        return null_argument("event");
    }
    let result = unsafe { handle_arg(subscription) }
        .and_then(Subscription::poll)
        // SAFETY: `event` is non-null and writable per the caller's contract.
        .map(|polled| unsafe { event.write(EventView::new(&polled)) });
    Status::from(result)
}

// --- Reader ---

/// An input read from a reader. Stays valid until the next poll.
#[repr(C)]
pub struct ReadInputView {
    id: InputIdView,
    /// How to read `buf`; "" if the input has no convention.
    payload_type: *const c_char,
    buf: *const u8,
    len: usize,
    /// How many inputs of the producer's session were lost right before this one, because the
    /// reader fell behind.
    lost: u64,
}

impl ReadInputView {
    fn new(input: &ReadInput<'_>) -> Self {
        Self {
            id: InputIdView {
                producer: input.producer.as_c_str(),
                session: input.id().session(),
                seq: input.id().seq(),
            },
            payload_type: input.envelope.payload_type.as_c_str(),
            buf: input.data().as_ptr(),
            len: input.data().len(),
            lost: input.lost(),
        }
    }
}

/// Reads the inputs that the producer `name` on this machine publishes, in order, alongside the
/// client that takes them, e.g. to record or display them. It reads every input, whatever its
/// flow. The producer need not exist yet. Only inputs published from now on are read. Use and
/// destroy the reader only on the thread that created it.
///
/// # Safety
/// `name` must be NUL-terminated; `reader` must be valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_reader_create(
    name: *const c_char,
    reader: *mut *mut Reader,
) -> Status {
    unsafe { create_handle(reader, || Reader::new(str_arg(name)?)) }
}

/// Destroys the reader. NULL is ignored.
///
/// # Safety
/// `reader` must be null or a live handle that is not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_reader_destroy(reader: *mut Reader) -> Status {
    unsafe { destroy_handle(reader) }
}

/// Waits until an input is ready, or `timeout_ms` milliseconds pass, and returns `ERR_AGAIN` then.
/// A timeout of 0 checks once without waiting. Fetch the input with `uriel_reader_poll`.
///
/// # Safety
/// `reader` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_reader_wait(reader: *mut Reader, timeout_ms: u32) -> Status {
    let timeout = Duration::from_millis(u64::from(timeout_ms));
    Status::from(unsafe { handle_arg(reader) }.and_then(|r| r.wait(timeout)))
}

/// Writes the next input to `*input`, or returns `ERR_AGAIN` if none is available. The input stays
/// valid until the next poll.
///
/// # Safety
/// `reader` must be a live handle; `input` must be valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uriel_reader_poll(
    reader: *mut Reader,
    input: *mut ReadInputView,
) -> Status {
    if input.is_null() {
        return null_argument("input");
    }
    let result = unsafe { handle_arg(reader) }
        .and_then(Reader::poll)
        // SAFETY: `input` is non-null and writable per the caller's contract.
        .map(|polled| unsafe { input.write(ReadInputView::new(&polled)) });
    Status::from(result)
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;

    use super::*;
    use crate::test_util::unique;

    #[test]
    fn producer_session_matches_its_inputs() {
        let name = CString::new(unique("producer")).unwrap();
        let mut producer = ptr::null_mut();
        let created =
            unsafe { uriel_producer_create(name.as_ptr(), ptr::null(), 8, &mut producer) };
        assert_eq!(created, Status::Ok);
        let mut session = 0;
        assert_eq!(
            unsafe { uriel_producer_session(producer, &mut session) },
            Status::Ok
        );

        let handle = unsafe { &mut *producer };
        handle.new_input().unwrap();
        let id = handle.publish_input(0).unwrap();
        assert_eq!(id.session(), session);
        assert_eq!(unsafe { uriel_producer_destroy(producer) }, Status::Ok);
    }

    #[test]
    fn destroy_null_is_ok() {
        assert_eq!(
            unsafe { uriel_producer_destroy(ptr::null_mut()) },
            Status::Ok
        );
    }

    #[test]
    fn create_rejects_null_out_pointer() {
        let status =
            unsafe { uriel_producer_create(c"p".as_ptr(), ptr::null(), 0, ptr::null_mut()) };
        assert_eq!(status, Status::ErrInvalid);
    }

    #[test]
    fn add_flow_rejects_a_malformed_spec() {
        let name = CString::new(unique("client")).unwrap();
        let server = CString::new(unique("server")).unwrap();
        let mut client = ptr::null_mut();
        let created = unsafe {
            uriel_client_create(name.as_ptr(), server.as_ptr(), ptr::null(), &mut client)
        };
        assert_eq!(created, Status::Ok);

        let status =
            unsafe { uriel_client_add_flow(client, c"flow".as_ptr(), c"[[edges]".as_ptr()) };

        assert_eq!(status, Status::ErrInvalid);
        assert_eq!(unsafe { uriel_client_destroy(client) }, Status::Ok);
    }

    /// Creates a client named after `name` with `options`, and destroys it if that worked.
    fn create_client(options: *const ClientOptions) -> Status {
        let name = CString::new(unique("client")).unwrap();
        let server = CString::new(unique("server")).unwrap();
        let mut client = ptr::null_mut();
        let status =
            unsafe { uriel_client_create(name.as_ptr(), server.as_ptr(), options, &mut client) };
        assert_eq!(unsafe { uriel_client_destroy(client) }, Status::Ok);
        status
    }

    fn client_options() -> *mut ClientOptions {
        let mut options = ptr::null_mut();
        assert_eq!(
            unsafe { uriel_client_options_create(&mut options) },
            Status::Ok
        );
        options
    }

    #[test]
    fn crossing_capacity_needs_a_remote_server() {
        let options = client_options();
        let set = unsafe { uriel_client_options_set_remote_crossing_capacity(options, 4) };
        assert_eq!(set, Status::Ok);

        assert_eq!(create_client(options), Status::ErrInvalid);
        assert_eq!(unsafe { uriel_client_options_destroy(options) }, Status::Ok);
    }

    #[test]
    fn zero_crossing_capacity_is_rejected() {
        let options = client_options();
        let set = unsafe {
            uriel_client_options_set_remote_server(
                options,
                c"127.0.0.1:4433".as_ptr(),
                ptr::null(),
                0,
            )
        };
        assert_eq!(set, Status::Ok);
        unsafe { uriel_client_options_set_remote_crossing_capacity(options, 0) };

        assert_eq!(create_client(options), Status::ErrInvalid);
        unsafe { uriel_client_options_destroy(options) };
    }

    #[test]
    fn remote_server_address_is_checked_when_set() {
        let options = client_options();

        let set = unsafe {
            uriel_client_options_set_remote_server(
                options,
                c"not an address".as_ptr(),
                ptr::null(),
                0,
            )
        };

        assert_eq!(set, Status::ErrInvalid);
        unsafe { uriel_client_options_destroy(options) };
    }

    #[test]
    fn identity_dir_needs_a_bind_address() {
        let mut options = ptr::null_mut();
        unsafe { uriel_server_options_create(&mut options) };
        unsafe { uriel_server_options_set_identity_dir(options, c"/tmp".as_ptr()) };
        let name = CString::new(unique("server")).unwrap();
        let mut server = ptr::null_mut();

        let status = unsafe { uriel_server_create(name.as_ptr(), options, &mut server) };

        assert_eq!(status, Status::ErrInvalid);
        assert!(server.is_null());
        unsafe { uriel_server_options_destroy(options) };
    }

    #[test]
    fn add_producer_gives_a_subscription_if_asked() {
        let client_name = CString::new(unique("client")).unwrap();
        let server_name = CString::new(unique("server")).unwrap();
        let mut client = ptr::null_mut();
        let created = unsafe {
            uriel_client_create(
                client_name.as_ptr(),
                server_name.as_ptr(),
                ptr::null(),
                &mut client,
            )
        };
        assert_eq!(created, Status::Ok);
        let producer_name = CString::new(unique("camera")).unwrap();
        let other_name = CString::new(unique("camera")).unwrap();
        let mut subscription = ptr::null_mut();

        let with =
            unsafe { uriel_client_add_producer(client, producer_name.as_ptr(), &mut subscription) };
        let without =
            unsafe { uriel_client_add_producer(client, other_name.as_ptr(), ptr::null_mut()) };

        assert_eq!((with, without), (Status::Ok, Status::Ok));
        assert!(!subscription.is_null());
        let mut event = std::mem::MaybeUninit::<EventView>::uninit();
        assert_eq!(
            unsafe { uriel_subscription_poll(subscription, event.as_mut_ptr()) },
            Status::ErrAgain
        );
        assert_eq!(
            unsafe { uriel_subscription_destroy(subscription) },
            Status::Ok
        );
        assert_eq!(unsafe { uriel_client_destroy(client) }, Status::Ok);
    }

    #[test]
    fn failed_add_producer_gives_no_subscription() {
        let client_name = CString::new(unique("client")).unwrap();
        let server_name = CString::new(unique("server")).unwrap();
        let mut client = ptr::null_mut();
        unsafe {
            uriel_client_create(
                client_name.as_ptr(),
                server_name.as_ptr(),
                ptr::null(),
                &mut client,
            )
        };
        let name = CString::new(unique("camera")).unwrap();
        let mut first = ptr::null_mut();
        unsafe { uriel_client_add_producer(client, name.as_ptr(), &mut first) };
        let mut second = ptr::null_mut();

        let again = unsafe { uriel_client_add_producer(client, name.as_ptr(), &mut second) };

        assert_eq!(again, Status::ErrInvalid);
        assert!(second.is_null());
        unsafe { uriel_subscription_destroy(first) };
        unsafe { uriel_client_destroy(client) };
    }

    #[test]
    fn abi_version_matches_the_header() {
        assert_eq!(uriel_abi_version(), ABI_VERSION);
    }

    #[test]
    fn subscription_without_events_is_again() {
        let name = CString::new(unique("camera")).unwrap();
        let mut subscription = ptr::null_mut();
        let created = unsafe { uriel_subscribe_producer(name.as_ptr(), &mut subscription) };
        assert_eq!(created, Status::Ok);
        let mut event = std::mem::MaybeUninit::<EventView>::uninit();

        let polled = unsafe { uriel_subscription_poll(subscription, event.as_mut_ptr()) };

        assert_eq!(polled, Status::ErrAgain);
        assert_eq!(
            unsafe { uriel_subscription_destroy(subscription) },
            Status::Ok
        );
    }

    #[test]
    fn reader_reads_a_published_input() {
        let name = CString::new(unique("camera")).unwrap();
        let mut reader = ptr::null_mut();
        let created = unsafe { uriel_reader_create(name.as_ptr(), &mut reader) };
        assert_eq!(created, Status::Ok);
        let mut input = std::mem::MaybeUninit::<ReadInputView>::uninit();
        assert_eq!(
            unsafe { uriel_reader_poll(reader, input.as_mut_ptr()) },
            Status::ErrAgain
        );

        let mut producer = Producer::new(name.to_str().unwrap(), None, 8).unwrap();
        let buf = producer.new_input().unwrap();
        buf[0].write(42);
        let id = producer.publish_input(1).unwrap();
        assert_eq!(unsafe { uriel_reader_wait(reader, 200) }, Status::Ok);
        let polled = unsafe { uriel_reader_poll(reader, input.as_mut_ptr()) };

        assert_eq!(polled, Status::Ok);
        let input = unsafe { input.assume_init() };
        assert_eq!(
            unsafe { CStr::from_ptr(input.id.producer) },
            name.as_c_str()
        );
        assert_eq!((input.id.session, input.id.seq), (id.session(), id.seq()));
        assert_eq!(
            unsafe { std::slice::from_raw_parts(input.buf, input.len) },
            [42]
        );
        assert_eq!(input.lost, 0);
        assert_eq!(unsafe { uriel_reader_destroy(reader) }, Status::Ok);
    }

    #[test]
    fn last_error_says_what_failed() {
        let mut producer = ptr::null_mut();
        unsafe { uriel_producer_create(ptr::null(), ptr::null(), 0, &mut producer) };

        let message = unsafe { CStr::from_ptr(uriel_last_error()) };

        assert_eq!(message, c"invalid argument: null string");
    }

    #[test]
    fn create_rejects_null_name() {
        let mut producer = ptr::null_mut();
        let status = unsafe { uriel_producer_create(ptr::null(), ptr::null(), 0, &mut producer) };
        assert_eq!(status, Status::ErrInvalid);
        assert!(producer.is_null());
    }
}
