//! Viewer coordination of raw remote loads.

pub mod client;
use crate::session::LoadResult;
use volna_trace::remote::transport::Packet;
pub enum ClientStep {
    Ack(Packet),
    Complete { ack: Packet, result: LoadResult },
    Yield,
}
