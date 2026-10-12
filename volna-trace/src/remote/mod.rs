//! Raw complete-object transport.

pub mod activity;
pub mod client;
mod decode;
mod equality;
pub mod hierarchy;
pub mod history;
pub mod limits;
pub mod memory;
pub mod metadata;
pub mod objects;
pub mod open;
pub mod session;
pub mod signals;
pub mod track_transfer;
pub mod tracks;
pub mod transport;

use crate::session::LoadResult;
use transport::Packet;

fn serialize_bytes<S: serde::Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_bytes(bytes)
}

pub enum ClientStep<Tag = u64> {
    Ack(Packet),
    Complete {
        ack: Packet,
        result: LoadResult<Tag>,
    },
    Yield,
}

#[cfg(test)]
mod tests {
    use bincode::Options;
    use serde::Serialize;

    #[test]
    fn bulk_bytes_preserve_the_fixed_bincode_wire_representation() {
        #[derive(Serialize)]
        struct Bulk<'a>(#[serde(serialize_with = "super::serialize_bytes")] &'a [u8]);
        for size in [0, 1, 255, 65536, super::transport::DATA_BYTES] {
            let bytes: Vec<_> = (0..size).map(|i| i as u8).collect();
            let opts = || {
                bincode::DefaultOptions::new()
                    .with_fixint_encoding()
                    .with_little_endian()
            };
            assert_eq!(
                opts().serialize(&Bulk(&bytes)).unwrap(),
                opts().serialize(&bytes).unwrap()
            );
        }
    }
}
