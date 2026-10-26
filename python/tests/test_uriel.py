import itertools
import os
import signal
import threading
import time

import numpy as np
import pytest

import uriel

MAX_INPUT_SIZE = 64
MAX_OUTPUT_SIZE = 64
FLOW = "flow"
# The type of the fixture producer's inputs.
INPUT_TYPE = "example.Frame"
# Far longer than anything here takes, so a test fails rather than hangs.
WAIT = 5.0
# How long a test lets a wait run before interrupting it.
INTERRUPT_AFTER = 0.2

_names = itertools.count()


def unique(base: str) -> str:
    """A name no other test uses, in this process or another."""
    return f"{base}-{os.getpid()}-{next(_names)}"


def chain(producer: str, engine: str) -> str:
    """A flow where `producer` feeds `engine`."""
    return f"""
[[producers]]
name = "{producer}"
tokens = 1

[[engines]]
name = "{engine}"
location = "local"

[[edges]]
from = "{producer}"
to = "{engine}"
timeout = "5s"
"""


class Fixture:
    """A local server serving one engine, a client of it, and a producer the client takes inputs
    from."""

    def __init__(self) -> None:
        self.producer_name, self.engine_name = unique("camera"), unique("detector")
        server_name = unique("server")
        self.server = uriel.Server(server_name)
        self.engine = uriel.Engine(self.engine_name, max_output_size=MAX_OUTPUT_SIZE)
        self.server.add_engine(self.engine_name)
        self.client = uriel.Client(unique("client"), server_name)
        self.producer = uriel.Producer(
            self.producer_name, input_type=INPUT_TYPE, max_input_size=MAX_INPUT_SIZE
        )
        self.client.add_producer(self.producer_name)

    def add_flow(self) -> None:
        self.client.add_flow(FLOW, chain(self.producer_name, self.engine_name))

    def close(self) -> None:
        for handle in (self.producer, self.client, self.engine, self.server):
            handle.close()


@pytest.fixture
def fixture():
    f = Fixture()
    yield f
    f.close()


def test_add_producer_with_events_reads_the_producers_events(fixture):
    other_name = unique("camera")
    with (
        fixture.client.add_producer_with_events(other_name) as events,
        uriel.Producer(other_name, max_input_size=MAX_INPUT_SIZE) as other,
    ):
        fixture.client.add_flow(unique("flow"), chain(other_name, fixture.engine_name))

        seq = other.send(b"frame")

        deadline = time.monotonic() + WAIT
        while (event := events.next(deadline - time.monotonic())).kind != uriel.EventKind.SENT:
            pass
        assert (event.id.producer, event.id.seq) == (other_name, seq)


def test_input_reaches_engine_and_result_reaches_app(fixture):
    with uriel.Subscription.producer(fixture.producer_name) as events:
        fixture.add_flow()

        seq = fixture.producer.send(b"frame")
        input = fixture.engine.next(WAIT)
        assert bytes(input.data) == b"frame"
        assert input.id.producer == fixture.producer_name
        assert input.id.session == fixture.producer.session
        assert input.id.seq == seq
        assert input.payload_type == INPUT_TYPE
        fixture.engine.forward(result=b"cat")

        deadline = time.monotonic() + WAIT
        while (event := events.next(deadline - time.monotonic())).kind != uriel.EventKind.RESULT:
            pass
        assert event.engine == fixture.engine_name
        assert event.id.seq == seq
        assert event.outcome == uriel.Outcome.FORWARD
        assert event.result == b"cat"
        assert 0 < event.at_ns <= time.monotonic_ns()


def test_loaned_input_is_published_in_place(fixture):
    fixture.add_flow()

    buf = fixture.producer.loan()
    assert len(buf) == MAX_INPUT_SIZE
    buf[:8] = b"in place"
    fixture.producer.publish(8)

    assert bytes(fixture.engine.next(WAIT).data) == b"in place"


def test_numpy_reads_an_input_without_copying_it(fixture):
    fixture.add_flow()
    fixture.producer.send(np.arange(8, dtype=np.uint8))

    input = fixture.engine.next(WAIT)
    frame = np.frombuffer(input.data, dtype=np.uint8)

    assert frame.tolist() == list(range(8))
    assert not frame.flags.owndata


def test_input_cannot_be_read_after_its_outcome(fixture):
    fixture.add_flow()
    fixture.producer.send(b"frame")
    input = fixture.engine.next(WAIT)

    fixture.engine.drop()

    with pytest.raises(ValueError):
        bytes(input.data)


def test_error_says_what_failed(fixture):
    with pytest.raises(uriel.InvalidArgumentError, match="invalid argument: "):
        fixture.client.add_flow(FLOW, "[[edges]")


def test_oversized_input_is_refused(fixture):
    with pytest.raises(uriel.SizeExceededError):
        fixture.producer.send(bytes(MAX_INPUT_SIZE + 1))


def test_engine_without_input_times_out(fixture):
    assert fixture.engine.next(0.01) is None
    assert fixture.engine.poll() is None


def test_ctrl_c_interrupts_a_waiting_engine(fixture):
    timer = threading.Timer(INTERRUPT_AFTER, os.kill, (os.getpid(), signal.SIGINT))
    timer.start()
    start = time.monotonic()

    with pytest.raises(KeyboardInterrupt):
        fixture.engine.next()

    assert time.monotonic() - start < WAIT
    timer.join()


def test_engine_on_a_thread_of_its_own_gets_inputs():
    server_name, engine_name, producer_name = unique("server"), unique("upper"), unique("app")
    ready, results = threading.Event(), []

    def serve():
        with uriel.Engine(engine_name, max_output_size=MAX_OUTPUT_SIZE) as engine:
            ready.set()
            input = engine.next(WAIT)
            results.append(bytes(input.data))
            engine.drop()

    thread = threading.Thread(target=serve)
    with uriel.Server(server_name) as server:
        thread.start()
        assert ready.wait(WAIT)
        server.add_engine(engine_name)
        with (
            uriel.Producer(producer_name, max_input_size=MAX_INPUT_SIZE) as producer,
            uriel.Client(unique("client"), server_name) as client,
        ):
            client.add_producer(producer_name)
            client.add_flow(FLOW, chain(producer_name, engine_name))
            producer.send(b"frame")
            thread.join(WAIT)

    assert results == [b"frame"]


def test_engine_used_from_another_thread_fails(fixture):
    errors = []
    thread = threading.Thread(target=lambda: errors.append(_raised(fixture.engine.poll)))
    thread.start()
    thread.join()

    assert isinstance(errors[0], uriel.WrongThreadError)


def _raised(function) -> Exception | None:
    try:
        function()
    except uriel.UrielError as e:
        return e
    return None


def test_closed_handle_fails():
    producer = uriel.Producer(unique("camera"), max_input_size=MAX_INPUT_SIZE)
    producer.close()
    producer.close()

    with pytest.raises(uriel.ClosedError):
        producer.loan()


def test_remote_server_gives_its_port_and_certificate():
    with uriel.Server(unique("server"), bind_addr="127.0.0.1:0") as server:
        assert server.remote_port > 0
        assert len(server.certificate) > 0


def test_client_takes_a_remote_server_and_a_crossing_capacity():
    with uriel.Server(unique("server"), bind_addr="127.0.0.1:0") as server:
        remote = uriel.RemoteServer(
            f"127.0.0.1:{server.remote_port}", server.certificate, crossing_capacity=4
        )
        with uriel.Client(unique("client"), unique("local"), remote=remote):
            pass


def test_negative_crossing_capacity_is_invalid():
    remote = uriel.RemoteServer("127.0.0.1:4433", b"", crossing_capacity=-1)
    with pytest.raises(uriel.InvalidArgumentError):
        uriel.Client(unique("client"), unique("local"), remote=remote)


def test_identity_dir_without_bind_addr_is_invalid(tmp_path):
    with pytest.raises(uriel.InvalidArgumentError):
        uriel.Server(unique("server"), identity_dir=tmp_path)


def test_an_event_kind_from_a_newer_abi_stays_an_int():
    from uriel.types import _known

    assert _known(uriel.EventKind, uriel.EventKind.RESULT) is uriel.EventKind.RESULT
    assert _known(uriel.EventKind, 1000) == 1000


def test_replacement_reaches_the_next_engine(fixture):
    tracker_name = unique("tracker")
    with uriel.Engine(tracker_name, max_output_size=MAX_OUTPUT_SIZE) as tracker:
        fixture.server.add_engine(tracker_name)
        fixture.client.add_flow(
            FLOW,
            chain(fixture.producer_name, fixture.engine_name)
            + f"""
[[engines]]
name = "{tracker_name}"
location = "local"

[[edges]]
from = "{fixture.engine_name}"
to = "{tracker_name}"
timeout = "5s"
""",
        )

        fixture.producer.send(b"frame")
        assert fixture.engine.next(WAIT) is not None
        output = fixture.engine.loan()
        output[:5] = b"tiles"
        fixture.engine.replace(5, output_type="example.Tiles")

        input = tracker.next(WAIT)
        assert bytes(input.data) == b"tiles"
        assert input.payload_type == "example.Tiles"
        tracker.drop()


def test_engine_subscriber_sees_the_engines_result(fixture):
    with uriel.Subscription.engine(fixture.engine_name) as events:
        fixture.add_flow()

        seq = fixture.producer.send(b"frame")
        assert fixture.engine.next(WAIT) is not None
        fixture.engine.forward(result=b"cat")

        deadline = time.monotonic() + WAIT
        while (event := events.next(deadline - time.monotonic())).kind != uriel.EventKind.RESULT:
            assert event.engine == fixture.engine_name
        assert event.id.seq == seq
        assert event.result == b"cat"


def test_engine_waits_for_an_event_of_an_engine_it_subscribes_to(fixture):
    with uriel.Engine(unique("fusion"), max_output_size=MAX_OUTPUT_SIZE) as fusion:
        fusion.subscribe(fixture.engine_name)
        fixture.add_flow()

        seq = fixture.producer.send(b"frame")
        assert fixture.engine.next(WAIT) is not None
        fixture.engine.forward(result=b"cat")

        deadline = time.monotonic() + WAIT
        while True:
            assert fusion.wait(deadline - time.monotonic())
            event = fusion.poll_event()
            if event is not None and event.kind == uriel.EventKind.RESULT:
                break
        assert event.engine == fixture.engine_name
        assert event.id.seq == seq
        assert event.result == b"cat"
        assert fusion.poll() is None


def test_fusion_publishes_a_derived_result_its_subscribers_get(fixture):
    fusion_name = unique("fusion")
    with (
        uriel.Engine(fusion_name, max_output_size=MAX_OUTPUT_SIZE) as fusion,
        uriel.Subscription.engine(fusion_name) as decisions,
    ):
        fixture.server.add_engine(fusion_name)
        fusion.subscribe(fixture.engine_name)
        fixture.add_flow()
        fixture.producer.send(b"frame")
        assert fixture.engine.next(WAIT) is not None
        fixture.engine.forward(result=b"cat")
        deadline = time.monotonic() + WAIT
        while (event := fusion.poll_event()) is None or event.kind != uriel.EventKind.RESULT:
            fusion.wait(deadline - time.monotonic())

        fusion.publish_derived(event.id, event.flow, b"person", result_type="example.Decision")

        decision = decisions.next(WAIT)
        assert decision.kind == uriel.EventKind.DERIVED
        assert decision.engine == fusion_name
        assert decision.id == event.id
        assert decision.result == b"person"
        assert decision.payload_type == "example.Decision"


def test_reader_and_engine_both_get_the_input(fixture):
    with uriel.Reader(fixture.producer_name) as reader:
        fixture.add_flow()

        seq = fixture.producer.send(b"frame")
        input = reader.next(WAIT)

        assert input.id == uriel.InputId(fixture.producer_name, fixture.producer.session, seq)
        assert (bytes(input.data), input.payload_type) == (b"frame", INPUT_TYPE)
        assert input.lost == 0
        assert fixture.engine.next(WAIT) is not None


def test_reader_reads_inputs_in_order():
    name = unique("camera")
    with (
        uriel.Reader(name) as reader,
        uriel.Producer(name, max_input_size=MAX_INPUT_SIZE) as producer,
    ):
        producer.send(b"one")
        producer.send(b"two")

        assert bytes(reader.next(WAIT).data) == b"one"
        assert bytes(reader.next(WAIT).data) == b"two"
        assert reader.poll() is None


def test_read_input_cannot_be_read_after_the_next_poll():
    name = unique("camera")
    with (
        uriel.Reader(name) as reader,
        uriel.Producer(name, max_input_size=MAX_INPUT_SIZE) as producer,
    ):
        producer.send(b"frame")
        input = reader.next(WAIT)

        reader.poll()

        with pytest.raises(ValueError):
            bytes(input.data)


def test_reader_without_input_times_out():
    with uriel.Reader(unique("camera")) as reader:
        assert reader.next(0.01) is None
