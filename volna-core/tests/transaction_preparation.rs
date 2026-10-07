//! Large distinct-lane and distinct-name recordings must remain practical to prepare.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use volna_core::data::loaded_tracks::LoadedGenerator;
use volna_core::pipeline::palette::StagePalette;
use volna_trace::data::transactions::*;

fn prepare(distinct_lanes: bool) {
    const COUNT: usize = 65_536;
    let tx = Transaction {
        id: TransactionRef(1),
        generator: TrackRef(1),
        begin: 0,
        end: 10,
        status: TxStatus::Ok,
        kind: TxKind::Unspecified,
        parent: None,
        attributes: vec![],
        events: vec![],
        stages: (0..COUNT)
            .map(|i| TransactionStage {
                name: if distinct_lanes {
                    "stage".into()
                } else {
                    format!("stage-{i}")
                },
                lane: if distinct_lanes {
                    format!("lane-{i}")
                } else {
                    "0".into()
                },
                begin: 0,
                end: Some(10),
                attributes: vec![],
            })
            .collect(),
    };
    let started = Instant::now();
    let generator =
        Arc::new(LoadedGenerator::new(TrackRef(1), vec![tx], HashMap::new(), vec![]).unwrap());
    let palette = StagePalette::build(std::slice::from_ref(&generator));
    let elapsed = started.elapsed();
    // This is a coarse stall guard, not a throughput benchmark. A 65k-entry
    // census requires billions of comparisons with repeated linear searches;
    // indexed preparation normally finishes far below this generous ceiling.
    assert!(
        elapsed < Duration::from_secs(5),
        "preparing 65k distinct entries stalled for {elapsed:?}"
    );
    if distinct_lanes {
        assert_eq!(generator.stage_census().lanes.len(), COUNT);
        assert_eq!(palette.names(), ["stage"]);
    } else {
        assert_eq!(palette.names().len(), COUNT);
        assert_eq!(palette.names()[0], "stage-0");
        assert_eq!(palette.names()[COUNT - 1], format!("stage-{}", COUNT - 1));
    }
}

#[test]
fn distinct_lanes_do_not_stall_preparation() {
    prepare(true);
}
#[test]
fn distinct_names_do_not_stall_preparation() {
    prepare(false);
}
