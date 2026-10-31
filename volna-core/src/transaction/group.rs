//! Bounded pages of point records selected from a waveform group.

use crate::data::loaded_tracks::LoadedGenerator;
use crate::data::points::{Counts, severity};
use crate::data::text::format_attribute;
use crate::document::RecordRange;
use volna_trace::data::transactions::{Transaction, TransactionRef};

pub const PAGE_SIZE: usize = 12;
pub const GROUP_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GroupQuery {
    pub search: String,
    pub severity: Option<usize>,
    pub page: usize,
}

#[derive(Clone, Debug)]
pub struct GroupRecord {
    pub id: TransactionRef,
    pub time: u64,
    pub severity: usize,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct GroupView {
    pub range: RecordRange,
    pub total: usize,
    pub counts: Counts,
    pub matching: usize,
    pub page: usize,
    pub records: Vec<GroupRecord>,
}

impl GroupView {
    #[must_use]
    pub fn build(
        generator: &LoadedGenerator,
        range: RecordRange,
        query: &GroupQuery,
    ) -> Option<Self> {
        let index = generator.points()?;
        let all = generator.transactions();
        let first = all.partition_point(|r| r.begin < range.begin);
        let end = all.partition_point(|r| r.begin <= range.end);
        if first >= end {
            return None;
        }
        let counts = index.counts(first..end);
        let source = &all[first..end];
        let search = query.search.to_lowercase();
        let matches = |record: &&Transaction| {
            if query
                .severity
                .is_some_and(|level| severity(record) != level)
            {
                return false;
            }
            search.is_empty()
                || record.id.0.to_string().contains(&search)
                || record.begin.to_string().contains(&search)
                || record.attributes.iter().any(|a| {
                    format_attribute(&a.value, 1024)
                        .to_lowercase()
                        .contains(&search)
                })
        };
        let (matching, page, selected) = if search.is_empty() {
            let matching = query.severity.map_or(source.len(), |level| {
                counts.get(level).copied().unwrap_or(0) as usize
            });
            let page = query.page.min(matching.saturating_sub(1) / PAGE_SIZE);
            let selected = match query.severity {
                None => source
                    .iter()
                    .skip(page * PAGE_SIZE)
                    .take(PAGE_SIZE)
                    .collect(),
                Some(level) => (page * PAGE_SIZE..((page + 1) * PAGE_SIZE).min(matching))
                    .map(|nth| {
                        let (mut low, mut high) = (first, end);
                        while low < high {
                            let mid = low + (high - low) / 2;
                            if index.counts(first..mid + 1)[level] > nth as u64 {
                                high = mid;
                            } else {
                                low = mid + 1;
                            }
                        }
                        &all[low]
                    })
                    .collect(),
            };
            (matching, page, selected)
        } else {
            search_page(source.iter().filter(matches), query.page)
        };
        let records = selected
            .into_iter()
            .map(|record| GroupRecord {
                id: record.id,
                time: record.begin,
                severity: severity(record),
                label: index.kind.label(record).map_or_else(
                    || format!("#{}", record.id.0),
                    |value| format_attribute(value, 256),
                ),
            })
            .collect();
        Some(Self {
            range,
            total: source.len(),
            counts,
            matching,
            page,
            records,
        })
    }
}

#[must_use]
pub fn bounded_search(text: &str) -> String {
    let mut end = text.len().min(256);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

// Keep the last page so an out-of-range request can be clamped without rescanning previews.
fn search_page<'a>(
    records: impl Iterator<Item = &'a Transaction>,
    requested_page: usize,
) -> (usize, usize, Vec<&'a Transaction>) {
    let mut matching = 0;
    let mut selected = Vec::with_capacity(PAGE_SIZE);
    let mut last_page = Vec::with_capacity(PAGE_SIZE);
    for record in records {
        if matching / PAGE_SIZE == requested_page {
            selected.push(record);
        }
        if matching % PAGE_SIZE == 0 {
            last_page.clear();
        }
        last_page.push(record);
        matching += 1;
    }
    let page = requested_page.min(matching.saturating_sub(1) / PAGE_SIZE);
    if page != requested_page {
        selected = last_page;
    }
    (matching, page, selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use volna_trace::data::transactions::{
        AttributeValue, SpanKind, Status, TrackRef, TransactionAttribute,
    };

    #[test]
    fn indexed_severity_pages_match_filtered_records_and_clamp_to_the_last_page() {
        let records = (0..2000)
            .map(|id| Transaction {
                id: TransactionRef(id),
                generator: TrackRef(1),
                begin: id / 4,
                end: id / 4,
                kind: SpanKind::Unspecified,
                status: Status::Ok,
                parent: None,
                attributes: vec![
                    TransactionAttribute {
                        key: "record.kind".into(),
                        value: AttributeValue::Text("log".into()),
                    },
                    TransactionAttribute {
                        key: "log.severity".into(),
                        value: AttributeValue::Enum {
                            value: i128::from(id % 6),
                            name: String::new(),
                        },
                    },
                    TransactionAttribute {
                        key: "log.message".into(),
                        value: AttributeValue::Text(format!("packet {} café", id % 17)),
                    },
                ],
                events: vec![],
                stages: vec![],
            })
            .collect();
        let loaded =
            LoadedGenerator::new(TrackRef(1), records, Default::default(), vec![]).unwrap();
        let range = RecordRange {
            begin: 10,
            end: 400,
        };
        for severity_filter in [None, Some(0), Some(4), Some(6), Some(99)] {
            for search in ["", "café", "packet 3", "absent"] {
                let expected: Vec<_> = loaded
                    .transactions()
                    .iter()
                    .filter(|r| {
                        (range.begin..=range.end).contains(&r.begin)
                            && severity_filter.is_none_or(|s| severity(r) == s)
                            && (search.is_empty()
                                || format_attribute(
                                    loaded.points().unwrap().kind.label(r).unwrap(),
                                    1024,
                                )
                                .contains(search))
                    })
                    .map(|r| r.id)
                    .collect();
                for page in [0, 1, 10, usize::MAX] {
                    let view = GroupView::build(
                        &loaded,
                        range,
                        &GroupQuery {
                            search: search.into(),
                            severity: severity_filter,
                            page,
                        },
                    )
                    .unwrap();
                    assert_eq!(view.matching, expected.len());
                    let page = page.min(expected.len().saturating_sub(1) / PAGE_SIZE);
                    assert_eq!(view.page, page);
                    assert_eq!(
                        view.records.iter().map(|r| r.id).collect::<Vec<_>>(),
                        expected
                            .iter()
                            .skip(page * PAGE_SIZE)
                            .take(PAGE_SIZE)
                            .copied()
                            .collect::<Vec<_>>()
                    );
                }
            }
        }
        assert!(
            GroupView::build(
                &loaded,
                RecordRange {
                    begin: 500,
                    end: 600
                },
                &GroupQuery::default()
            )
            .is_none()
        );
        assert!(
            GroupView::build(
                &loaded,
                RecordRange {
                    begin: 100,
                    end: 10
                },
                &GroupQuery::default()
            )
            .is_none()
        );

        let mut record = loaded.transactions()[0].clone();
        record
            .attributes
            .iter_mut()
            .find(|a| a.key == "log.message")
            .unwrap()
            .value = AttributeValue::Text("é".repeat(200));
        let loaded =
            LoadedGenerator::new(TrackRef(1), vec![record], Default::default(), vec![]).unwrap();
        let query = GroupQuery {
            search: bounded_search(&"é".repeat(200)),
            ..Default::default()
        };
        let view = GroupView::build(&loaded, RecordRange { begin: 0, end: 0 }, &query).unwrap();
        assert_eq!(
            view.matching, 1,
            "a capped Unicode query must retain its literal prefix"
        );
        assert_eq!(view.records[0].id, TransactionRef(0));
    }
}
