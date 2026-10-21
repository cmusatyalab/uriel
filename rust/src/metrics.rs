//! Prometheus metrics of what only Uriel knows: how a client uses its tokens and crossings, and
//! how its connection to the remote server is doing. The events about inputs show the rest, and
//! apps record metrics from them themselves.
//!
//! Uriel records them through the [`metrics`] facade, so they cost almost nothing unless the
//! app installs a recorder. [`serve`] installs one that serves them to Prometheus. A Rust app
//! that installs a recorder of its own gets them there too.
//!
//! - `uriel_tokens`, `uriel_tokens_in_use`: a producer's tokens in a flow, and how many of
//!   them its inputs hold. All in use means the producer waits for the flow.
//! - `uriel_crossing_capacity`, `uriel_crossing_in_flight`: how many inputs a crossing may
//!   have in flight, and how many it has. A remote crossing that is full waits for the network or
//!   the remote engines.
//! - `uriel_remote_connected`: 1 while the client is connected to the remote server.
//! - `uriel_remote_rtt_seconds`: the connection's smoothed round trip time.
//! - `uriel_remote_congestion_window_bytes`: how much the connection may have in flight.
//! - `uriel_remote_lost_packets_total`, `uriel_remote_congestion_events_total`.
//! - `uriel_remote_sent_bytes_total`, `uriel_remote_received_bytes_total`: UDP bytes.
//!
//! The token metrics have `flow` and `producer_id` labels, the crossing metrics `flow`,
//! `location` and `targets` (the crossing's engines), and the remote metrics `server`.

use std::net::SocketAddr;

use metrics_exporter_prometheus::PrometheusBuilder;

use crate::{Error, Result};

pub(crate) const TOKENS: &str = "uriel_tokens";
pub(crate) const TOKENS_IN_USE: &str = "uriel_tokens_in_use";
pub(crate) const CROSSING_CAPACITY: &str = "uriel_crossing_capacity";
pub(crate) const CROSSING_IN_FLIGHT: &str = "uriel_crossing_in_flight";
pub(crate) const REMOTE_CONNECTED: &str = "uriel_remote_connected";
pub(crate) const REMOTE_RTT: &str = "uriel_remote_rtt_seconds";
pub(crate) const REMOTE_CONGESTION_WINDOW: &str = "uriel_remote_congestion_window_bytes";
pub(crate) const REMOTE_LOST_PACKETS: &str = "uriel_remote_lost_packets_total";
pub(crate) const REMOTE_CONGESTION_EVENTS: &str = "uriel_remote_congestion_events_total";
pub(crate) const REMOTE_SENT_BYTES: &str = "uriel_remote_sent_bytes_total";
pub(crate) const REMOTE_RECEIVED_BYTES: &str = "uriel_remote_received_bytes_total";

/// Histogram buckets, in seconds, for apps that record latencies through the same recorder: from
/// a fast engine on a GPU to one behind a slow link.
pub const LATENCY_BUCKETS: &[f64] = &[
    0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// Serves the metrics recorded in this process at `http://<addr>/metrics`, on a thread of the
/// exporter's own. Call it before creating clients and servers: metrics they register before it
/// are not recorded. It fails if a recorder is already installed.
pub fn serve(addr: SocketAddr) -> Result<()> {
    PrometheusBuilder::new()
        .with_http_listener(addr)
        .set_buckets(LATENCY_BUCKETS)
        .map_err(|e| Error::Internal(format!("invalid histogram buckets: {e}")))?
        .install()
        .map_err(|e| Error::Internal(format!("failed to serve metrics on {addr}: {e}")))
}
