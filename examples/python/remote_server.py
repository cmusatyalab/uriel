"""A Uriel server with one engine, for remote clients such as the Android example.

    uv run python ../examples/python/remote_server.py --identity-dir ~/uriel/identity

The server accepts remote clients over QUIC. The engine "hello-upper" upper-cases each input, and
reports it as its result. The first time the server starts, it makes its certificate in the identity
directory (`certificate.der`); give that file to the clients, which trust only it.
"""

import argparse
import signal
import threading
from pathlib import Path

import uriel

DEFAULT_LISTEN_ADDR = "0.0.0.0:4433"
ENGINE_NAME = "hello-upper"
MAX_OUTPUT_SIZE = 1024
# How often the engine's wait ends, in seconds, so that it notices a request to stop.
POLL_INTERVAL = 0.5


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--name", default="uriel", help="the server's name")
    parser.add_argument(
        "--listen", default=DEFAULT_LISTEN_ADDR, help="address to accept clients on"
    )
    parser.add_argument(
        "--identity-dir", type=Path, required=True, help="where the certificate and key are kept"
    )
    args = parser.parse_args()

    stopping = threading.Event()
    for signum in (signal.SIGINT, signal.SIGTERM):
        signal.signal(signum, lambda *_: stopping.set())

    args.identity_dir.mkdir(parents=True, exist_ok=True)
    with (
        uriel.Server(args.name, bind_addr=args.listen, identity_dir=args.identity_dir) as server,
        uriel.Engine(ENGINE_NAME, max_output_size=MAX_OUTPUT_SIZE) as engine,
    ):
        server.add_engine(ENGINE_NAME)
        print(
            f"server {args.name} serves {ENGINE_NAME} on {args.listen}; "
            f"clients trust {args.identity_dir / 'certificate.der'}",
            flush=True,
        )
        while not stopping.is_set():
            input = engine.next(POLL_INTERVAL)
            if input is not None:
                print(f"input {input.id.seq} from {input.id.producer}", flush=True)
                engine.drop(result=bytes(input.data).upper())


if __name__ == "__main__":
    main()
