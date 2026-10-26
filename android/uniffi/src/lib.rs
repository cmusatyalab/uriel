//! Uriel for Android: a client that sends an app's inputs to a remote server over QUIC, and the
//! events about them. Kotlin and Java reach it through the bindings that UniFFI generates from
//! this file.
//!
//! There is no local server and there are no local engines: everything here stays in the app's
//! process (uriel's `in-process` feature), and the flows' engines all run on the remote server.
//! An input is written by a [`Producer`], which the [`Client`] takes by name; what happened to it
//! is read from a [`Subscription`].
//!
//! Inputs and results cross into Kotlin as copies, as `ByteArray`s. Numbers are signed 64-bit
//! (`Long`), because Java cannot call the unsigned types that Kotlin gives `u64`. Sizes, counts and
//! times are never negative; a session is a random number, reinterpreted as signed.

use std::{
    net::SocketAddr,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

uniffi::setup_scaffolding!();

/// The name of the local server that the client would use. Nothing serves it, so flows can only
/// have remote engines.
const NO_LOCAL_SERVER: &str = "uriel-android-no-local-server";

/// What went wrong in a call. The fields are not named `message`, which would clash with the
/// Kotlin exception's own.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum UrielError {
    /// An argument was invalid, or the call is not allowed in the object's state.
    #[error("invalid argument: {reason}")]
    InvalidArgument { reason: String },
    /// An input was larger than the producer's maximum input size.
    #[error("size {size} exceeds maximum of {max}")]
    SizeExceeded { size: i64, max: i64 },
    /// Something failed inside Uriel, e.g. the network.
    #[error("internal error: {reason}")]
    Internal { reason: String },
}

impl From<uriel::Error> for UrielError {
    fn from(error: uriel::Error) -> Self {
        match error {
            uriel::Error::InvalidArgument(reason) => Self::InvalidArgument { reason },
            uriel::Error::SizeExceeded { size, max } => Self::SizeExceeded {
                size: to_i64(size),
                max: to_i64(max),
            },
            // Only `Subscription::next` waits, and it turns "nothing yet" into no event.
            uriel::Error::Again => Self::Internal {
                reason: error.to_string(),
            },
            uriel::Error::Internal(reason) => Self::Internal { reason },
        }
    }
}

type Result<T> = std::result::Result<T, UrielError>;

/// `value` as the signed number Java sees. Sizes and counts are far below `i64::MAX`.
fn to_i64(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// A non-negative `value` of the argument `name`, as a size.
fn to_usize(name: &str, value: i64) -> Result<usize> {
    usize::try_from(value).map_err(|_| UrielError::InvalidArgument {
        reason: format!("{name} {value} is negative or too large"),
    })
}

/// Locks `mutex`. A panic in another call does not make this one fail.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The remote server a client sends to.
#[derive(Debug, Clone, uniffi::Record)]
pub struct RemoteServer {
    /// The server's IP address and port, e.g. "192.0.2.1:4433" or "[2001:db8::1]:4433". A host
    /// name is not accepted.
    pub addr: String,
    /// The server's certificate in DER. The client trusts this certificate only.
    pub certificate: Vec<u8>,
    /// How many inputs may be in flight on each edge of a flow that crosses to the server; at
    /// least 1. `null` for the default.
    pub crossing_capacity: Option<i64>,
}

impl RemoteServer {
    fn config(self) -> Result<uriel::RemoteServerConfig> {
        let addr: SocketAddr = self.addr.parse().map_err(|e| UrielError::InvalidArgument {
            reason: format!("invalid server address {:?}: {e}", self.addr),
        })?;
        let mut config = uriel::RemoteServerConfig::new(addr, self.certificate);
        if let Some(capacity) = self.crossing_capacity {
            config.crossing_capacity = to_usize("crossing capacity", capacity)?;
        }
        Ok(config)
    }
}

/// Bridges the app's producers to the remote server that runs a flow's engines. It may be used
/// from any thread.
#[derive(uniffi::Object)]
pub struct Client {
    inner: Mutex<uriel::Client>,
}

#[uniffi::export]
impl Client {
    /// Creates the client `name`, which connects to `remote` on its own, and connects again after
    /// a loss. The server may start later.
    #[uniffi::constructor]
    pub fn new(name: String, remote: RemoteServer) -> Result<Arc<Self>> {
        let client = uriel::Client::new(&name, NO_LOCAL_SERVER, Some(remote.config()?))?;
        Ok(Arc::new(Self {
            inner: Mutex::new(client),
        }))
    }

    /// Starts taking the inputs of the producer `name`. The producer may be created before or
    /// after this call.
    pub fn add_producer(&self, name: String) -> Result<()> {
        Ok(lock(&self.inner).add_producer(&name)?)
    }

    /// Starts taking the inputs of the producer `name`, and returns a subscription to the events
    /// about them. It exists before the client takes the first input, so it misses no event.
    pub fn add_producer_with_events(&self, name: String) -> Result<Arc<Subscription>> {
        let events = lock(&self.inner).add_producer_with_events(&name)?;
        Ok(Arc::new(Subscription {
            inner: Mutex::new(events),
        }))
    }

    /// Stops taking the inputs of the producer `name`. Fails while a flow uses it.
    pub fn remove_producer(&self, name: String) -> Result<()> {
        Ok(lock(&self.inner).remove_producer(&name)?)
    }

    /// Adds the flow `name`. `spec` describes it in TOML: its producers, which the client must
    /// take already; its engines, all with `location = "remote"`; and the edges between them.
    pub fn add_flow(&self, name: String, spec: String) -> Result<()> {
        let spec = spec.parse()?;
        Ok(lock(&self.inner).add_flow(&name, &spec)?)
    }

    /// Removes the flow `name`.
    pub fn remove_flow(&self, name: String) -> Result<()> {
        Ok(lock(&self.inner).remove_flow(&name)?)
    }
}

/// Publishes the app's inputs. The client sends each one through every flow that lists the
/// producer.
#[derive(uniffi::Object)]
pub struct Producer {
    inner: Mutex<uriel::Producer>,
    max_input_size: usize,
}

#[uniffi::export]
impl Producer {
    /// Creates the producer `name`, whose inputs are at most `max_input_size` bytes. `input_type`
    /// names how to read them, e.g. a protobuf message name; engines see it as each input's
    /// payload type.
    #[uniffi::constructor]
    pub fn new(name: String, input_type: Option<String>, max_input_size: i64) -> Result<Arc<Self>> {
        let max_input_size = to_usize("maximum input size", max_input_size)?;
        let producer = uriel::Producer::new(&name, input_type.as_deref(), max_input_size)?;
        Ok(Arc::new(Self {
            inner: Mutex::new(producer),
            max_input_size,
        }))
    }

    /// Publishes `data` as the next input, and returns its sequence number: 1 for the first input,
    /// then one more for each. Events about the input carry it. Fails with `SizeExceeded` if
    /// `data` is larger than the producer's maximum input size.
    pub fn send(&self, data: Vec<u8>) -> Result<i64> {
        if data.len() > self.max_input_size {
            return Err(UrielError::SizeExceeded {
                size: to_i64(data.len()),
                max: to_i64(self.max_input_size),
            });
        }
        let mut producer = lock(&self.inner);
        producer.new_input()?[..data.len()].write_copy_of_slice(&data);
        Ok(producer.publish_input(data.len())?.seq() as i64)
    }

    /// The producer's session: a random number chosen when it was created. Input ids carry it
    /// with the sequence number, so they stay unique when a producer of the same name restarts.
    pub fn session(&self) -> i64 {
        lock(&self.inner).session() as i64
    }
}

/// What happened, as a subscription reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum EventKind {
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
    /// The client lost its connection to a server of the flow `flow`. It names no input.
    Disconnected,
    /// A result an engine derived from the events it subscribes to. It answers no offer.
    Derived,
    /// Events were lost before the next one, because the subscriber fell behind.
    Lost,
}

impl From<uriel::EventKind> for EventKind {
    fn from(kind: uriel::EventKind) -> Self {
        match kind {
            uriel::EventKind::Accepted => Self::Accepted,
            uriel::EventKind::Sent => Self::Sent,
            uriel::EventKind::Result => Self::Result,
            uriel::EventKind::Skipped => Self::Skipped,
            uriel::EventKind::TimedOut => Self::TimedOut,
            uriel::EventKind::Disconnected => Self::Disconnected,
            uriel::EventKind::Derived => Self::Derived,
        }
    }
}

/// What an engine did with an input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Outcome {
    /// The engine forwarded nothing.
    Drop,
    /// The engine forwarded the input unchanged.
    Forward,
    /// The engine forwarded a new input in place of the input.
    Replace,
}

impl From<uriel::Outcome> for Outcome {
    fn from(outcome: uriel::Outcome) -> Self {
        match outcome {
            uriel::Outcome::Drop => Self::Drop,
            uriel::Outcome::Forward => Self::Forward,
            uriel::Outcome::Replace => Self::Replace,
        }
    }
}

/// An event read from a subscription. For `Lost`, only `lost` is set; for `Disconnected`, only
/// `flow`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct Event {
    pub kind: EventKind,
    /// For `Lost`: how many events were lost.
    pub lost: i64,
    /// The name of the producer that published the input the event is about.
    pub producer: String,
    /// The producer's session (`Producer.session`).
    pub session: i64,
    /// The input's sequence number within the session, from 1.
    pub seq: i64,
    /// The engine the event is about; empty if none.
    pub engine: String,
    /// The flow the event belongs to; empty if none.
    pub flow: String,
    /// What the engine did with the input, for `Result`.
    pub outcome: Outcome,
    /// How to read `result`; empty if there is no result.
    pub payload_type: String,
    pub result: Vec<u8>,
    /// When the event happened, in nanoseconds on the remote server's clock.
    pub at_ns: i64,
    /// When the server made the engine's offer, on the same clock; 0 for `Disconnected`.
    pub offered_at_ns: i64,
    /// When the client received the event from the server, in nanoseconds on this device's
    /// monotonic clock.
    pub received_at_ns: i64,
}

impl Event {
    fn lost(count: u64) -> Self {
        Self {
            kind: EventKind::Lost,
            lost: count as i64,
            producer: String::new(),
            session: 0,
            seq: 0,
            engine: String::new(),
            flow: String::new(),
            outcome: Outcome::Drop,
            payload_type: String::new(),
            result: Vec::new(),
            at_ns: 0,
            offered_at_ns: 0,
            received_at_ns: 0,
        }
    }

    fn published(event: &uriel::subscription::Published<'_>) -> Self {
        let id = event.id();
        Self {
            kind: event.kind().into(),
            lost: 0,
            producer: id.producer().to_owned(),
            session: id.session() as i64,
            seq: id.seq() as i64,
            engine: event.engine().to_owned(),
            flow: event.flow().to_owned(),
            outcome: event.outcome().into(),
            payload_type: event.payload_type().to_owned(),
            result: event.result().to_vec(),
            at_ns: event.at().as_nanos() as i64,
            offered_at_ns: event.offered_at().map_or(0, |at| at.as_nanos() as i64),
            received_at_ns: event.received_at().map_or(0, |at| at.as_nanos() as i64),
        }
    }
}

/// Reads the events about the inputs of one producer: what happened to each at each engine, and
/// lost connections. It may be used from any thread, one call at a time.
#[derive(uniffi::Object)]
pub struct Subscription {
    inner: Mutex<uriel::Subscription>,
}

#[uniffi::export]
impl Subscription {
    /// Waits for the next event for at most `timeout_ms` milliseconds. Returns `null` if none
    /// came. The call blocks, so run it off the main thread.
    pub fn next(&self, timeout_ms: i64) -> Result<Option<Event>> {
        let timeout = Duration::from_millis(to_usize("timeout", timeout_ms)? as u64);
        let mut events = lock(&self.inner);
        if let Some(event) = poll(&mut events)? {
            return Ok(Some(event));
        }
        match events.wait(timeout) {
            Ok(()) => poll(&mut events),
            Err(uriel::Error::Again) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

/// The next event of `events`, or `None` if none is available.
fn poll(events: &mut uriel::Subscription) -> Result<Option<Event>> {
    match events.poll() {
        Ok(uriel::subscription::Event::Published(event)) => Ok(Some(Event::published(&event))),
        Ok(uriel::subscription::Event::Lost(lost)) => Ok(Some(Event::lost(lost.count()))),
        Err(uriel::Error::Again) => Ok(None),
        Err(e) => Err(e.into()),
    }
}
