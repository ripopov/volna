//! Cooperative decoding and validation of raw metadata.

use super::attributes::{Values, attributes};
use super::decode::{Decoder, Reader, Step};
use super::memory::{MemoryBudget, Reservation};
use crate::data::TraceInfo;
use crate::data::source::Generator;
use crate::data::transactions::{Track, TrackRef};
use crate::session::Capabilities;

pub struct MetadataDecoder(Decoder<super::hierarchy::Header>);

pub struct ValidatedMetadata {
    pub(super) header: super::hierarchy::Header,
    pub(super) reservation: Reservation,
}

pub enum MetadataStep {
    NeedInput,
    Yield,
    Decoded(Box<ValidatedMetadata>),
}

impl MetadataDecoder {
    pub fn new(declared: u64, limit: u64, budget: &MemoryBudget) -> anyhow::Result<Self> {
        Ok(Self(Decoder::new(declared, limit, budget, |reader| {
            Box::pin(async move { metadata(&reader).await })
        })?))
    }
    pub fn feed(&mut self, bytes: Vec<u8>) -> anyhow::Result<()> {
        self.0.feed(bytes)
    }
    pub fn step(&mut self) -> anyhow::Result<MetadataStep> {
        Ok(match self.0.step()? {
            Step::NeedInput => MetadataStep::NeedInput,
            Step::Yield => MetadataStep::Yield,
            Step::Ready(metadata, reservation) => {
                MetadataStep::Decoded(Box::new(ValidatedMetadata {
                    header: metadata,
                    reservation,
                }))
            }
        })
    }
}

async fn metadata(r: &Reader) -> anyhow::Result<super::hierarchy::Header> {
    let values = Values::read(r).await?;
    let info = TraceInfo {
        name: r.string().await?,
        design_id: if r.boolean().await? {
            Some(r.string().await?)
        } else {
            None
        },
        timescale: r.u8().await? as i8,
        time_range: (r.u64().await?, r.u64().await?),
        signal_count: r.usize().await?,
        change_count: if r.boolean().await? {
            Some(r.u64().await?)
        } else {
            None
        },
        time_unit: if r.boolean().await? {
            Some(r.string().await?)
        } else {
            None
        },
    };
    let capabilities = Capabilities {
        waveforms: r.boolean().await?,
        transactions: r.boolean().await?,
        relations: r.boolean().await?,
    };
    let tracks = r.vector(24, || track(r, &values)).await?;
    let generators = r
        .vector(28, || async {
            Ok(Generator {
                name: r.string().await?,
                scope: r.usize().await?,
                root: r.boolean().await?,
                kind: r.string().await?,
                track: TrackRef(r.u32().await?),
                attributes: attributes(r, &values, 0).await?,
            })
        })
        .await?;
    let enum_tables = r
        .vector(12, || async {
            Ok(crate::data::source::EnumTable {
                id: r.u32().await?,
                entries: r
                    .vector(16, || async { Ok((r.string().await?, r.string().await?)) })
                    .await?,
            })
        })
        .await?;
    let mut table_ids = std::collections::HashSet::new();
    r.charge(enum_tables.len() * 32)?;
    for table in &enum_tables {
        r.checkpoint().await;
        anyhow::ensure!(table_ids.insert(table.id), "duplicate enum table");
    }
    let scopes = r.u32().await?;
    let vars = r.u32().await?;
    let activity = if r.boolean().await? {
        let descriptor = super::activity::Descriptor {
            format: r.u8().await?,
            length: r.u64().await?,
            toc_crc: r.u32().await?,
            recording: {
                let mut recording = [0; 16];
                r.read(&mut recording).await?;
                recording
            },
            available: r.boolean().await?,
        };
        descriptor.identity()?;
        Some(descriptor)
    } else {
        None
    };
    let server = r.string().await?;
    Ok(super::hierarchy::Header {
        info,
        capabilities,
        tracks,
        generators,
        enum_tables,
        scopes,
        vars,
        activity,
        server,
    })
}

async fn track(r: &Reader, values: &Values) -> anyhow::Result<Track> {
    Ok(Track {
        id: TrackRef(r.u32().await?),
        path: r.vector(8, || r.string()).await?,
        kind: r.string().await?,
        attributes: attributes(r, values, 0).await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::transactions::AttributeValue;
    use crate::remote::hierarchy::Header;
    use crate::remote::objects::Metadata;
    use crate::remote::transport::DATA_BYTES;

    fn sample() -> Metadata {
        let source = crate::testing::ProceduralTrace::session(100);
        let mut metadata = Metadata::from_session(source.as_ref());
        metadata.info.design_id = Some("build λ".into());
        use AttributeValue::*;
        let values = vec![
            Null,
            Bool(true),
            I64(-7),
            U64(u64::MAX),
            F64(f64::NAN),
            Text("aλ𐀀".repeat(DATA_BYTES / 4)),
            Bytes(vec![0, 255]),
            Logic {
                width: 2,
                states: 9,
                data: vec![0x87],
            },
            Time(17),
            Enum {
                value: -2,
                name: "enum".into(),
            },
            Enum {
                value: i128::from(u64::MAX),
                name: "maximum".into(),
            },
            Pointer(u64::MAX),
            Fixed {
                raw: -3,
                scale: i64::MIN,
            },
            UFixed {
                raw: 3,
                scale: i64::MAX,
            },
            List(vec![Bool(false)]),
            Map(vec![("key".into(), I64(4))]),
            AttributeValue::shared(Map(vec![("mnemonic".into(), Text("FADD".into()))])),
        ];
        metadata.tracks = vec![Track {
            id: TrackRef(0),
            path: vec!["top".into()],
            kind: "raw".into(),
            attributes: values
                .into_iter()
                .map(|value| ("attr".into(), value))
                .collect(),
        }];
        metadata.capabilities.transactions = true;
        metadata
    }

    #[test]
    fn metadata_roundtrip_matches_bincode_across_fragmented_utf8_and_all_attributes() {
        let expected = sample();
        roundtrip(
            Header::from_session(crate::testing::ProceduralTrace::session(100).as_ref())
                .map(|mut header| {
                    header.enum_tables = vec![crate::data::source::EnumTable {
                        id: 42,
                        entries: vec![("空闲.λ".into(), "00".into()), ("BUSY".into(), "01".into())],
                    }];
                    header.info = expected.info;
                    header.tracks = expected.tracks;
                    header.capabilities = expected.capabilities;
                    header.generators = expected.hierarchy.generators.as_ref().clone();
                    header.activity = Some(super::super::activity::Descriptor {
                        format: 1,
                        length: 123_456,
                        toc_crc: 0x1234_abcd,
                        recording: std::array::from_fn(|index| (index * 13) as u8),
                        available: true,
                    });
                    header
                })
                .unwrap(),
        );
    }

    #[test]
    fn malformed_logic_attributes_are_rejected_and_release_admission() {
        for (width, states, data) in [
            (u64::MAX, 2, vec![]),
            (9, 2, vec![1]),
            (1, 4, vec![0x80]),
            (1, 9, vec![9]),
            (1, 3, vec![0]),
        ] {
            let mut header =
                Header::from_session(crate::testing::ProceduralTrace::session(1).as_ref()).unwrap();
            header.tracks.push(Track {
                id: TrackRef(0),
                path: vec!["raw".into()],
                kind: "OWNER".into(),
                attributes: vec![(
                    "logic".into(),
                    AttributeValue::Logic {
                        width,
                        states,
                        data,
                    },
                )],
            });
            let bytes = bincode::serialize(&header).unwrap();
            let budget = MemoryBudget::new(1 << 20);
            let mut decoder = MetadataDecoder::new(bytes.len() as u64, 1 << 20, &budget).unwrap();
            decoder.feed(bytes).unwrap();
            loop {
                match decoder.step() {
                    Err(error) => {
                        assert!(error.to_string().contains("logic"));
                        break;
                    }
                    Ok(MetadataStep::Yield) => {}
                    Ok(_) => panic!("malformed logic was not rejected"),
                }
            }
            drop(decoder);
            assert_eq!(budget.used(), 0);
        }
    }

    fn roundtrip(expected: Header) {
        let bytes = bincode::serialize(&expected).unwrap();
        for size in [1, 7, DATA_BYTES] {
            let budget = MemoryBudget::new(16 * 1024 * 1024);
            let mut decoder =
                MetadataDecoder::new(bytes.len() as u64, bytes.len() as u64, &budget).unwrap();
            let mut chunks = bytes.chunks(size);
            let (metadata, reservation) = loop {
                match decoder.step().unwrap() {
                    MetadataStep::NeedInput => decoder
                        .feed(chunks.next().expect("input remaining").to_vec())
                        .unwrap(),
                    MetadataStep::Yield => {}
                    MetadataStep::Decoded(decoded) => {
                        break (decoded.header, decoded.reservation);
                    }
                }
            };
            assert!(chunks.next().is_none());
            assert_eq!(bincode::serialize(&metadata).unwrap(), bytes);

            assert!(budget.used() > 0);
            drop(metadata);
            drop(reservation);
            assert_eq!(budget.used(), 0);
        }
    }

    #[test]
    fn declared_lengths_cannot_allocate_past_remaining_input_or_memory_budget() {
        for length in [u64::MAX, 8192] {
            let budget = MemoryBudget::new(1024 * 1024);
            let declared = if length == u64::MAX { 8 } else { 8200 };
            let mut decoder = MetadataDecoder::new(declared, declared, &budget).unwrap();
            let held = budget.reserve(1024 * 1024 - budget.used()).unwrap();
            decoder.feed(length.to_le_bytes().to_vec()).unwrap();
            assert!(decoder.step().is_err());
            drop(decoder);
            drop(held);
            assert_eq!(budget.used(), 0);
        }
    }

    #[test]
    fn large_catalog_yields_even_when_all_input_is_available() {
        let mut metadata = sample();
        metadata.tracks[0].attributes.clear();
        let template = metadata.tracks[0].clone();
        metadata.tracks = (0..4000)
            .map(|id| Track {
                id: TrackRef(id),
                ..template.clone()
            })
            .collect();
        let bytes = bincode::serialize(&Header {
            info: metadata.info.clone(),
            capabilities: metadata.capabilities,
            tracks: metadata.tracks.clone(),
            generators: metadata.hierarchy.generators.as_ref().clone(),
            enum_tables: metadata.hierarchy.enum_tables().to_vec(),
            scopes: metadata.hierarchy.scope_count() as u32,
            vars: metadata.hierarchy.var_count() as u32,
            activity: metadata.activity.clone(),
            server: metadata.server.clone(),
        })
        .unwrap();
        assert!(bytes.len() < DATA_BYTES);
        let budget = MemoryBudget::new(16 * 1024 * 1024);
        let mut decoder =
            MetadataDecoder::new(bytes.len() as u64, bytes.len() as u64, &budget).unwrap();
        decoder.feed(bytes).unwrap();
        assert!(matches!(decoder.step().unwrap(), MetadataStep::Yield));
        let mut yields = 1;
        loop {
            match decoder.step().unwrap() {
                MetadataStep::Yield => yields += 1,
                MetadataStep::NeedInput => panic!("all input supplied"),
                MetadataStep::Decoded(decoded) => {
                    let metadata = decoded.header;
                    assert_eq!(metadata.tracks.len(), 4000);
                    break;
                }
            }
        }
        assert!(yields > 1);
    }

    #[test]
    fn malformed_header_never_decodes() {
        let header =
            Header::from_session(crate::testing::ProceduralTrace::session(10).as_ref()).unwrap();
        let valid = bincode::serialize(&header).unwrap();
        let mut trailing = valid.clone();
        trailing.push(0);
        let mut utf8 = valid.clone();
        utf8[16] = 255;
        for bytes in [valid[..valid.len() - 1].to_vec(), trailing, utf8] {
            let budget = MemoryBudget::new(4 * 1024 * 1024);
            let mut decoder =
                MetadataDecoder::new(bytes.len() as u64, bytes.len() as u64, &budget).unwrap();
            decoder.feed(bytes).unwrap();
            loop {
                match decoder.step() {
                    Err(_) => break,
                    Ok(MetadataStep::Yield) => {}
                    _ => panic!("malformed header accepted"),
                }
            }
            drop(decoder);
            assert_eq!(budget.used(), 0);
        }
    }

    #[test]
    fn shared_table_rejects_forward_references_and_releases_private_storage() {
        let header =
            Header::from_session(crate::testing::ProceduralTrace::session(1).as_ref()).unwrap();
        let valid = bincode::serialize(&header).unwrap();
        assert_eq!(&valid[..8], &0u64.to_le_bytes());
        for id in [0u32, 1, u32::MAX] {
            let mut bytes = 1u64.to_le_bytes().to_vec();
            bytes.extend_from_slice(&15u32.to_le_bytes());
            bytes.extend_from_slice(&id.to_le_bytes());
            bytes.extend_from_slice(&valid[8..]);
            let budget = MemoryBudget::new(1024 * 1024);
            let mut decoder =
                MetadataDecoder::new(bytes.len() as u64, 1024 * 1024, &budget).unwrap();
            decoder.feed(bytes).unwrap();
            loop {
                match decoder.step() {
                    Err(error) => {
                        assert!(error.to_string().contains("shared attribute reference"));
                        break;
                    }
                    Ok(MetadataStep::Yield) => (),
                    Ok(_) => panic!("accepted a reference to an unpublished table entry"),
                }
            }
            drop(decoder);
            assert_eq!(budget.used(), 0);
        }
    }
}
