//! Stage colours.

use std::collections::HashMap;
use std::sync::Arc;

use crate::color::Color;
use crate::data::loaded_tracks::{LoadedGenerator, StageCensus};
use crate::theme::Theme;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageStyle {
    pub fill: Color,
    pub edge: Color,
    pub text: Color,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageSwatch {
    Step { rank: usize, of: usize },
    Fallback,
}

pub const DEFAULT_LANE: &str = "0";

#[derive(Clone, Debug)]
pub struct StagePalette {
    by_name: HashMap<String, usize>,
    names: Vec<String>,
    primary_lane: String,
}

impl Default for StagePalette {
    fn default() -> Self {
        Self {
            by_name: HashMap::new(),
            names: Vec::new(),
            primary_lane: DEFAULT_LANE.to_owned(),
        }
    }
}

impl StagePalette {
    #[must_use]
    pub fn build(generators: &[Arc<LoadedGenerator>]) -> Self {
        Self::from_censuses(generators.iter().map(|g| g.stage_census()))
    }

    pub(crate) fn from_censuses<'a>(
        censuses: impl Iterator<Item = &'a StageCensus> + Clone,
    ) -> Self {
        let lanes = || censuses.clone().flat_map(|c| &c.lanes);
        let mut lane_counts: Vec<(&str, u64)> = Vec::new();
        let mut lane_index = HashMap::new();
        for lane in lanes() {
            let index = *lane_index.entry(lane.lane.as_str()).or_insert_with(|| {
                lane_counts.push((&lane.lane, 0));
                lane_counts.len() - 1
            });
            lane_counts[index].1 += lane.stages;
        }
        let primary_lane = if lane_counts.iter().any(|(lane, _)| *lane == DEFAULT_LANE) {
            DEFAULT_LANE.to_owned()
        } else {
            lane_counts
                .iter()
                .max_by_key(|(_, n)| *n)
                .map_or_else(|| DEFAULT_LANE.to_owned(), |(lane, _)| (*lane).to_owned())
        };
        let mut seen: Vec<(&str, u64, u64)> = Vec::new();
        let mut name_index = HashMap::new();
        for census in lanes()
            .filter(|lane| lane.lane == primary_lane)
            .flat_map(|lane| &lane.names)
        {
            let index = *name_index.entry(census.name.as_str()).or_insert_with(|| {
                seen.push((&census.name, 0, 0));
                seen.len() - 1
            });
            seen[index].1 += census.position_sum;
            seen[index].2 += census.count;
        }
        let mut order: Vec<usize> = (0..seen.len()).collect();
        order.sort_by(|&a, &b| {
            let mean = |i: usize| seen[i].1 as f64 / seen[i].2 as f64;
            mean(a).total_cmp(&mean(b))
        });
        let names: Vec<String> = order.into_iter().map(|i| seen[i].0.to_owned()).collect();
        let by_name = names
            .iter()
            .enumerate()
            .map(|(k, name)| (name.clone(), k))
            .collect();
        Self {
            by_name,
            names,
            primary_lane,
        }
    }

    pub(crate) fn resident_bytes(&self) -> u64 {
        (self.primary_lane.capacity()
            + self.names.capacity() * std::mem::size_of::<String>()
            + self.by_name.capacity() * (std::mem::size_of::<(String, usize)>() + 1)
            + self.names.iter().map(String::capacity).sum::<usize>()
            + self.by_name.keys().map(String::capacity).sum::<usize>()) as u64
    }

    #[must_use]
    pub fn swatch(&self, name: &str) -> StageSwatch {
        match self.by_name.get(name) {
            Some(&rank) => StageSwatch::Step {
                rank,
                of: self.names.len(),
            },
            None => StageSwatch::Fallback,
        }
    }

    #[must_use]
    pub fn style(&self, name: &str, theme: &Theme) -> StageStyle {
        theme.swatch(self.swatch(name))
    }

    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.names
    }

    #[must_use]
    pub fn primary_lane(&self) -> &str {
        &self.primary_lane
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use volna_trace::data::transactions::*;

    fn one_dark() -> Theme {
        Theme::one_dark()
    }

    fn generator(stages: &[&[(&str, &str)]]) -> Arc<LoadedGenerator> {
        let transactions = stages
            .iter()
            .enumerate()
            .map(|(i, stages)| Transaction {
                id: TransactionRef(i as u64),
                generator: TrackRef(1),
                begin: i as u64,
                end: i as u64 + 4,
                status: TxStatus::Unset,
                kind: TxKind::Unspecified,
                parent: None,
                attributes: vec![],
                events: vec![],
                stages: stages
                    .iter()
                    .enumerate()
                    .map(|(k, (name, lane))| TransactionStage {
                        name: (*name).into(),
                        lane: (*lane).into(),
                        begin: i as u64 + k as u64,
                        end: Some(i as u64 + k as u64 + 1),
                        attributes: vec![],
                    })
                    .collect(),
            })
            .collect();
        Arc::new(LoadedGenerator::new(TrackRef(1), transactions, HashMap::new(), vec![]).unwrap())
    }

    #[test]
    fn ladder_follows_pipeline_position_on_the_primary_lane() {
        let g = generator(&[
            &[("F", "0"), ("stl", "1"), ("X", "0")],
            &[("F", "0"), ("D", "0"), ("X", "0")],
        ]);
        let p = StagePalette::build(&[g]);
        assert_eq!(p.names(), ["F", "D", "X"]);
        assert_eq!(p.primary_lane(), "0");
        assert_ne!(p.style("F", &one_dark()), p.style("X", &one_dark()));
        assert_eq!(p.style("stl", &one_dark()), one_dark().stage_fallback());
        assert!(
            p.style("F", &one_dark()).fill.l > 0.5
                && p.style("F", &one_dark()).edge.l < p.style("F", &one_dark()).fill.l
        );
        let named = generator(&[&[("F", "main"), ("M", "memory")], &[("W", "main")]]);
        let p = StagePalette::build(&[named]);
        assert_eq!(p.primary_lane(), "main");
        assert_eq!(p.names(), ["F", "W"]);
        assert_eq!(StagePalette::default().primary_lane(), "0");
        let unnamed = generator(&[&[("F", ""), ("S", "stall"), ("X", "")]]);
        let p = StagePalette::build(&[unnamed]);
        assert_eq!(p.primary_lane(), "");
        assert_eq!(p.names(), ["F", "X"]);
    }

    #[test]
    fn a_dozen_stages_keep_pipeline_order_and_neighbours_apart() {
        let alu: &[(&str, &str)] = &[
            ("ID", ""),
            ("IR", ""),
            ("IS", ""),
            ("IQ", ""),
            ("RF", ""),
            ("EX", ""),
            ("CM", ""),
            ("RT", ""),
        ];
        let branch: &[(&str, &str)] = &[
            ("ID", ""),
            ("IR", ""),
            ("IS", ""),
            ("IQ", ""),
            ("RF", ""),
            ("BJ", ""),
            ("CM", ""),
            ("RT", ""),
        ];
        let load: &[(&str, &str)] = &[
            ("ID", ""),
            ("IR", ""),
            ("IS", ""),
            ("IQ", ""),
            ("RF", ""),
            ("AG", ""),
            ("DC", ""),
            ("DA", ""),
            ("WB", ""),
            ("CM", ""),
            ("RT", ""),
        ];
        let p = StagePalette::build(&[generator(&[alu, load, branch, alu])]);
        assert_eq!(
            p.names(),
            [
                "ID", "IR", "IS", "IQ", "RF", "EX", "AG", "BJ", "DC", "CM", "DA", "RT", "WB"
            ]
        );
        for pair in p.names().windows(2) {
            let (a, b) = (
                p.style(&pair[0], &one_dark()).fill,
                p.style(&pair[1], &one_dark()).fill,
            );
            assert!((a.l - b.l).abs() > 0.1, "{pair:?} differ in lightness");
            assert_ne!(a.h, b.h);
        }
        let few = StagePalette::build(&[generator(&[alu])]);
        assert!(
            few.names()
                .iter()
                .all(|n| few.style(n, &one_dark()).fill.l == 0.58)
        );
    }

    #[test]
    fn generators_merge_as_one_record_list() {
        let first: &[&[(&str, &str)]] = &[&[("F", "p"), ("X", "p")], &[("s", "q"), ("s", "q")]];
        let second: &[&[(&str, &str)]] = &[
            &[("F", "p"), ("D", "p"), ("X", "p")],
            &[("s", "q"), ("F", "p"), ("D", "p"), ("X", "p")],
        ];
        let merged = StagePalette::build(&[generator(first), generator(second)]);
        let together = [first, second].concat();
        let single = StagePalette::build(&[generator(&together)]);
        assert_eq!(merged.primary_lane(), "p");
        assert_eq!(merged.names(), ["F", "D", "X"]);
        assert_eq!(merged.names(), single.names());
        assert_eq!(
            merged.style("D", &one_dark()),
            single.style("D", &one_dark())
        );
    }
}
