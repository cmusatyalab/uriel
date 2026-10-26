//! The Android client against a remote server in the same process, as the app's tests would run
//! it: every service is in memory, and the engines are on the server.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use uriel::{Engine, Server};
use uriel_android::{
    Client, Event, EventKind, Outcome, Producer, RemoteServer, Subscription, UrielError,
};

const MAX_INPUT_SIZE: i64 = 64;
const MAX_OUTPUT_SIZE: usize = 64;
const INPUT_TYPE: &str = "example.Frame";
const FLOW: &str = "detect";
/// Far longer than anything here takes, so a test fails rather than hangs.
const WAIT: Duration = Duration::from_secs(5);

static NEXT_NAME: AtomicUsize = AtomicUsize::new(0);

fn unique(base: &str) -> String {
    format!("{base}-{}", NEXT_NAME.fetch_add(1, Ordering::Relaxed))
}

fn flow_spec(producer: &str, engine: &str) -> String {
    format!(
        r#"
[[producers]]
name = "{producer}"

[[engines]]
name = "{engine}"
location = "remote"

[[edges]]
from = "{producer}"
to = "{engine}"
timeout = "5s"
"#
    )
}

/// A remote server with one engine, a client of it, and a producer the client takes inputs from.
struct Fixture {
    _server: Server,
    engine: Engine,
    client: Arc<Client>,
    producer: Arc<Producer>,
    events: Arc<Subscription>,
}

impl Fixture {
    fn new() -> Self {
        let mut server = Server::new(&unique("remote"), Some("127.0.0.1:0"), None).unwrap();
        let engine_name = unique("detector");
        let engine = Engine::new(&engine_name, None, MAX_OUTPUT_SIZE).unwrap();
        server.add_engine(&engine_name).unwrap();
        let client = Client::new(
            unique("client"),
            RemoteServer {
                addr: server.remote_addr().unwrap().to_string(),
                certificate: server.certificate().unwrap().to_vec(),
                crossing_capacity: None,
            },
        )
        .unwrap();
        let producer_name = unique("camera");
        let producer = Producer::new(
            producer_name.clone(),
            Some(INPUT_TYPE.to_owned()),
            MAX_INPUT_SIZE,
        )
        .unwrap();
        let events = client
            .add_producer_with_events(producer_name.clone())
            .unwrap();
        client
            .add_flow(FLOW.to_owned(), flow_spec(&producer_name, &engine_name))
            .unwrap();
        Self {
            _server: server,
            engine,
            client,
            producer,
            events,
        }
    }

    /// Reads events until one of `kind` arrives.
    fn event(&self, kind: EventKind) -> Event {
        let deadline = Instant::now() + WAIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let event = self
                .events
                .next(left.as_millis() as i64)
                .unwrap()
                .expect("no event arrived");
            if event.kind == kind {
                return event;
            }
        }
    }
}

#[test]
fn input_reaches_a_remote_engine_and_its_result_comes_back() {
    let mut fixture = Fixture::new();

    let seq = fixture.producer.send(b"frame".to_vec()).unwrap();
    fixture.engine.wait(WAIT).unwrap();
    let input = fixture.engine.poll_input().unwrap();
    assert_eq!(input.data, b"frame");
    assert_eq!(input.payload_type(), INPUT_TYPE);
    fixture.engine.forward(b"cat").unwrap();

    let result = fixture.event(EventKind::Result);
    assert_eq!(
        (
            result.seq,
            result.session,
            result.outcome,
            result.flow.as_str()
        ),
        (seq, fixture.producer.session(), Outcome::Forward, FLOW)
    );
    assert_eq!(result.result, b"cat");
    assert!(result.at_ns > 0);
}

#[test]
fn objects_are_used_from_other_threads() {
    let mut fixture = Fixture::new();
    let producer = fixture.producer.clone();
    let events = fixture.events.clone();

    let seq = thread::spawn(move || producer.send(b"frame".to_vec()).unwrap())
        .join()
        .unwrap();
    fixture.engine.wait(WAIT).unwrap();
    fixture.engine.poll_input().unwrap();
    fixture.engine.drop(&[]).unwrap();

    let sent = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Some(event) = events.next(100).unwrap()
                && event.kind == EventKind::Sent
            {
                return Some(event.seq);
            }
        }
        None
    })
    .join()
    .unwrap();
    assert_eq!(sent, Some(seq));
}

#[test]
fn next_without_events_times_out() {
    let fixture = Fixture::new();

    assert!(fixture.events.next(10).unwrap().is_none());
}

#[test]
fn input_larger_than_the_maximum_is_refused() {
    let fixture = Fixture::new();

    let result = fixture.producer.send(vec![0; MAX_INPUT_SIZE as usize + 1]);

    assert!(matches!(
        result,
        Err(UrielError::SizeExceeded { size, max }) if size == MAX_INPUT_SIZE + 1 && max == MAX_INPUT_SIZE
    ));
}

#[test]
fn bad_server_address_is_refused() {
    let result = Client::new(
        unique("client"),
        RemoteServer {
            addr: String::from("not an address"),
            certificate: Vec::new(),
            crossing_capacity: None,
        },
    );

    assert!(matches!(result, Err(UrielError::InvalidArgument { .. })));
}

#[test]
fn flow_with_a_missing_producer_is_refused() {
    let fixture = Fixture::new();

    let result = fixture.client.add_flow(
        unique("flow"),
        flow_spec(&unique("absent"), &unique("detector")),
    );

    assert!(matches!(result, Err(UrielError::InvalidArgument { .. })));
}
