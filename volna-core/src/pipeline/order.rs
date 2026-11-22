//! Generic stage-aware row ordering for pipeline panels.

use std::collections::HashMap;

use super::palette::StagePalette;
use volna_trace::data::transactions::Transaction;

pub const NO_SORT_STAGE: u32 = u32::MAX;

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum RowOrderMode {
    #[default]
    Start,
    End,
    Stages {
        stages: Vec<String>,
    },
    FromStage {
        stage: String,
    },
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum UnmatchedPolicy {
    #[default]
    FallbackEnd,
    FallbackStart,
    Hide,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct RowOrder {
    #[serde(flatten)]
    pub mode: RowOrderMode,
    #[serde(default)]
    pub unmatched: UnmatchedPolicy,
    #[serde(default = "default_true")]
    pub follow_sort_key: bool,
}

impl Default for RowOrder {
    fn default() -> Self {
        Self {
            mode: RowOrderMode::Start,
            unmatched: UnmatchedPolicy::FallbackEnd,
            follow_sort_key: true,
        }
    }
}

impl RowOrder {
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    #[must_use]
    pub fn is_start(&self) -> bool {
        matches!(self.mode, RowOrderMode::Start)
    }

    #[must_use]
    pub fn is_stage_based(&self) -> bool {
        matches!(
            self.mode,
            RowOrderMode::Stages { .. } | RowOrderMode::FromStage { .. }
        )
    }

    #[must_use]
    pub fn short_label(&self) -> String {
        match &self.mode {
            RowOrderMode::Start => "Start".into(),
            RowOrderMode::End => "End".into(),
            RowOrderMode::Stages { stages } => match stages.as_slice() {
                [] => "Start".into(),
                [only] => only.clone(),
                [a, b] => format!("{a}, {b}"),
                [first, rest @ ..] => format!("{first} +{}", rest.len()),
            },
            RowOrderMode::FromStage { stage } => format!("{stage}+"),
        }
    }

    #[must_use]
    pub fn status_label(&self, unmatched_count: usize) -> Option<String> {
        if self.is_start() {
            return None;
        }
        let base = format!("Order: {}", self.short_label());
        if self.is_stage_based() && unmatched_count > 0 {
            let suffix = match self.unmatched {
                UnmatchedPolicy::FallbackEnd => "fallback→end",
                UnmatchedPolicy::FallbackStart => "fallback→start",
                UnmatchedPolicy::Hide => "hidden",
            };
            Some(format!("{base} ({unmatched_count} {suffix})"))
        } else {
            Some(base)
        }
    }

    /// Resolve the active target stage names without heap allocation.
    #[must_use]
    pub fn resolved_stages<'a>(&'a self, palette: &'a StagePalette) -> &'a [String] {
        match &self.mode {
            RowOrderMode::Start | RowOrderMode::End => &[],
            RowOrderMode::Stages { stages } => stages.as_slice(),
            RowOrderMode::FromStage { stage } => palette
                .names_from(stage)
                .unwrap_or_else(|| std::slice::from_ref(stage)),
        }
    }

    pub fn normalize(&mut self) {
        match &mut self.mode {
            RowOrderMode::Stages { stages } => {
                let mut deduped = Vec::with_capacity(stages.len());
                for name in stages.drain(..) {
                    if !name.is_empty() && !deduped.contains(&name) {
                        deduped.push(name);
                    }
                }
                if deduped.is_empty() {
                    self.mode = RowOrderMode::Start;
                } else {
                    *stages = deduped;
                }
            }
            RowOrderMode::FromStage { stage } => {
                if stage.is_empty() {
                    self.mode = RowOrderMode::Start;
                }
            }
            RowOrderMode::Start | RowOrderMode::End => {}
        }
    }

    #[must_use]
    pub fn valid(&self) -> bool {
        match &self.mode {
            RowOrderMode::Start | RowOrderMode::End => true,
            RowOrderMode::Stages { stages } => {
                !stages.is_empty() && stages.iter().all(|s| !s.is_empty())
            }
            RowOrderMode::FromStage { stage } => !stage.is_empty(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowOrderCommand {
    Set(RowOrder),
    SetMode(RowOrderMode),
    ToggleStage(String),
    SetUnmatched(UnmatchedPolicy),
    SetFollowSortKey(bool),
    Reset,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StageOrderOption {
    pub name: String,
    pub count: u64,
    pub total_transactions: u64,
    pub coverage: f32,
}

/// Compact per-row sort metadata retained only when a panel uses a non-`Start` row order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowSortMeta {
    pub sort_key: u64,
    pub sort_stage: u32,
    pub matched: bool,
}

impl RowSortMeta {
    #[must_use]
    pub fn sort_stage_index(self) -> Option<usize> {
        (self.sort_stage != NO_SORT_STAGE).then_some(self.sort_stage as usize)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EvaluatedRow {
    pub sort_key: u64,
    pub secondary_key: u64,
    pub sort_stage: u32,
    pub matched: bool,
}

pub(crate) fn evaluate_row_sort(
    tx: &Transaction,
    order: &RowOrder,
    resolved_stages: &[String],
    primary_lane: &str,
) -> Option<EvaluatedRow> {
    match &order.mode {
        RowOrderMode::Start => Some(EvaluatedRow {
            sort_key: tx.begin,
            secondary_key: tx.begin,
            sort_stage: NO_SORT_STAGE,
            matched: true,
        }),
        RowOrderMode::End => Some(EvaluatedRow {
            sort_key: tx.end,
            secondary_key: tx.begin,
            sort_stage: NO_SORT_STAGE,
            matched: true,
        }),
        RowOrderMode::Stages { .. } | RowOrderMode::FromStage { .. } => {
            let mut best: Option<(u64, u32)> = None;
            for (idx, stage) in tx.stages.iter().enumerate() {
                if stage.lane == primary_lane
                    && resolved_stages.iter().any(|target| target == &stage.name)
                {
                    let cand = (stage.begin, idx as u32);
                    if best.is_none_or(|prev| cand < prev) {
                        best = Some(cand);
                    }
                }
            }
            if let Some((begin, stage_idx)) = best {
                return Some(EvaluatedRow {
                    sort_key: begin,
                    secondary_key: tx.begin,
                    sort_stage: stage_idx,
                    matched: true,
                });
            }
            match order.unmatched {
                UnmatchedPolicy::Hide => None,
                UnmatchedPolicy::FallbackEnd => Some(EvaluatedRow {
                    sort_key: tx.end,
                    secondary_key: tx.begin,
                    sort_stage: NO_SORT_STAGE,
                    matched: false,
                }),
                UnmatchedPolicy::FallbackStart => Some(EvaluatedRow {
                    sort_key: tx.begin,
                    secondary_key: tx.begin,
                    sort_stage: NO_SORT_STAGE,
                    matched: false,
                }),
            }
        }
    }
}

/// Detect whether the active window shows batched same-source `tx.begin` bursts and, if so,
/// suggest an early high-coverage primary-lane stage that breaks those bursts cleanly.
pub(crate) fn detect_batch_suggestion(
    samples: &[(usize, &Transaction)],
    palette: &StagePalette,
    options: &[StageOrderOption],
) -> Option<String> {
    const MIN_SAMPLES: usize = 12;
    const BURST_MIN_COUNT: usize = 4;

    if samples.len() < MIN_SAMPLES || palette.names().len() < 2 {
        return None;
    }

    let mut begin_buckets: HashMap<(usize, u64), usize> = HashMap::new();
    for &(source, tx) in samples {
        *begin_buckets.entry((source, tx.begin)).or_insert(0) += 1;
    }
    let in_burst: usize = begin_buckets
        .values()
        .copied()
        .filter(|&c| c >= BURST_MIN_COUNT)
        .sum();
    if in_burst * 100 < samples.len() * 35 {
        return None;
    }

    let mut stage_buckets: HashMap<(usize, u64), usize> = HashMap::new();
    let mut best: Option<(usize, usize, f32, &str)> = None;
    for (rank, opt) in options.iter().enumerate() {
        if opt.coverage < 0.30 {
            continue;
        }
        stage_buckets.clear();
        let mut matched = 0usize;
        for &(source, tx) in samples {
            if let Some(st) = tx
                .stages
                .iter()
                .find(|s| s.lane == palette.primary_lane() && s.name == opt.name)
            {
                matched += 1;
                *stage_buckets.entry((source, st.begin)).or_insert(0) += 1;
            }
        }
        if matched * 100 < samples.len() * 40 {
            continue;
        }
        let stage_burst: usize = stage_buckets
            .values()
            .copied()
            .filter(|&c| c >= BURST_MIN_COUNT)
            .sum();
        if stage_burst * 4 <= in_burst && stage_buckets.len() >= begin_buckets.len() * 2 {
            // Prefer the earliest stage that breaks the burst cleanly; if two stages have
            // identical zero-burst dispersion, pick the earlier pipeline rank.
            let candidate = (stage_burst, rank, opt.coverage, opt.name.as_str());
            if best.is_none_or(|prev| {
                candidate.0 < prev.0 || (candidate.0 == prev.0 && candidate.1 < prev.1)
            }) {
                best = Some(candidate);
            }
            if stage_burst == 0 {
                break;
            }
        }
    }
    best.map(|(_, _, _, name)| name.to_owned())
}
