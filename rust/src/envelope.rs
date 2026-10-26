//! Payload envelope: metadata carried in each sample's iceoryx2 user header.

use iceoryx2::prelude::ZeroCopySend;

use crate::ipc::types::PayloadType;

/// Maximum length of a flow name, in bytes, not counting the terminating NUL.
pub const MAX_FLOW_NAME: usize = 64;

#[derive(Debug, Default, Clone, Copy, ZeroCopySend)]
#[repr(C)]
pub struct Envelope {
    /// The producer's session, chosen randomly when it starts. With `seq`, it identifies the input.
    pub session: u64,
    /// Increases with every input the producer publishes.
    pub seq: u64,
    /// Number of valid bytes at the start of the payload.
    pub len: u64,
    /// How to read the input, as its producer names it; empty if it has no convention.
    pub payload_type: PayloadType,
}
