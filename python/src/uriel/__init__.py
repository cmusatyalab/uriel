"""Python bindings for Uriel, through its C ABI (rust/include/uriel/uriel.h).

A producer publishes inputs. A client takes the producer's inputs and sends them through the flows
that list the producer to the flows' engines, which a server serves. Each engine processes one input at a time and reports one
outcome for it: `Engine.drop`, `Engine.forward` or `Engine.replace`. The app reads what happened to
its producer's inputs from a `Subscription`, and anyone can read a producer's inputs themselves with
a `Reader`.

A `Producer`, an `Engine`, a `Subscription` and a `Reader` must stay on the thread that created
them; using one from another thread raises `WrongThreadError`. A `Client` and a `Server` may be
used from any thread. Close every handle when done with it, or use it in a `with` block.
"""

from uriel._ffi import check as _check
from uriel._ffi import cstr as _cstr
from uriel._ffi import lib as _lib
from uriel.client import Client, RemoteServer
from uriel.engine import Engine
from uriel.errors import (
    AgainError,
    ClosedError,
    InternalError,
    InvalidArgumentError,
    SizeExceededError,
    UrielError,
    WrongThreadError,
)
from uriel.producer import Producer
from uriel.reader import Reader
from uriel.server import Server
from uriel.subscription import Subscription
from uriel.types import Event, EventKind, Input, InputId, Outcome, ReadInput


def init_log(filter: str | None = None) -> None:
    """Prints Uriel's log to standard error. `filter` picks what is printed, in the syntax of
    RUST_LOG, e.g. "info" or "uriel=debug". If it is None, RUST_LOG picks, or warnings and errors
    are printed if that is unset. Call it once, before anything else."""
    _check(_lib.uriel_log_init(_cstr(filter)))


def serve_metrics(addr: str) -> None:
    """Serves the metrics of Uriel's clients and servers in this process to Prometheus, at
    http://<addr>/metrics, e.g. "0.0.0.0:9464". Call it once, before creating clients and
    servers."""
    _check(_lib.uriel_metrics_serve(_cstr(addr)))


__all__ = [
    "AgainError",
    "Client",
    "ClosedError",
    "Engine",
    "Event",
    "EventKind",
    "Input",
    "InputId",
    "InternalError",
    "InvalidArgumentError",
    "Outcome",
    "Producer",
    "ReadInput",
    "Reader",
    "RemoteServer",
    "Server",
    "SizeExceededError",
    "Subscription",
    "UrielError",
    "WrongThreadError",
    "init_log",
    "serve_metrics",
]
