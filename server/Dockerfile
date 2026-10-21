# SPDX-FileCopyrightText: 2026 Carnegie Mellon University
#
# SPDX-License-Identifier: Apache-2.0

# Builds uriel-server. Build it from the repository's root, which holds the
# workspace:
#   docker build -f server/Dockerfile -t uriel-server .
# Engines in other containers reach the server through iceoryx2, so they share
# its /dev/shm and /tmp/iceoryx2. Give clients a copy of the certificate in its
# identity directory.

FROM rust:1.98-bookworm AS build
# iceoryx2's bindings are generated with bindgen, which needs libclang.
RUN apt-get update \
    && apt-get install -y --no-install-recommends libclang-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /uriel
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/uriel/target \
    cargo build --release --locked -p uriel-server \
    && cp target/release/uriel-server /usr/local/bin/

FROM debian:bookworm-slim
COPY --from=build /usr/local/bin/uriel-server /usr/local/bin/
EXPOSE 4433/udp
ENTRYPOINT ["uriel-server"]
