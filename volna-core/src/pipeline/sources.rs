//! Multi-generator pipeline source metadata, dynamic badge derivation for arbitrary VTR paths,
//! and heterogeneous stage-vocabulary detection.

use std::collections::HashSet;
use std::sync::Arc;

use super::model::TrackSource;
use crate::data::loaded_tracks::LoadedGenerator;
use crate::trace::Traced;
use volna_trace::data::transactions::TrackRef;

pub const MAX_ALIAS_CHARS: usize = 6;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusStyle {
    #[default]
    Dim,
    Isolate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipelineSource {
    pub track: TrackSource,
    pub visible: bool,
    pub alias: Option<String>,
    pub color_slot: usize,
}

impl PipelineSource {
    #[must_use]
    pub fn new(track: TrackSource, color_slot: usize) -> Self {
        Self {
            track,
            visible: true,
            alias: None,
            color_slot,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceLoadStatus {
    Ready { count: usize },
    Loading,
    Failed(String),
    Unresolved,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PipelineSourceCommand {
    Add(Traced<TrackRef>),
    Remove(usize),
    Reorder { from: usize, to: usize },
    ToggleVisible(usize),
    SetVisible(usize, bool),
    SetAlias(usize, Option<String>),
    ToggleFocus(usize),
    Isolate(usize),
    SetFocusStyle(FocusStyle),
    ClearFocusAndShowAll,
    Hover(Option<usize>),
    RetryFailed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceLabel {
    pub distinguishing: String,
    pub auto_badge: String,
    pub badge: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceBadges {
    pub panel_title: String,
    pub compact_title: String,
    pub labels: Vec<SourceLabel>,
    pub max_badge_chars: usize,
}

#[must_use]
pub fn clean_alias(raw: Option<&str>) -> Option<String> {
    let trimmed = raw?.trim();
    if trimmed.is_empty() {
        return None;
    }
    let clamped: String = trimmed.chars().take(MAX_ALIAS_CHARS).collect();
    (!clamped.is_empty()).then_some(clamped)
}

fn parse_indexed_segment(seg: &str) -> Option<(&str, &str)> {
    if let Some(open) = seg.rfind('[')
        && let Some(idx) = seg[open + 1..].strip_suffix(']')
        && !idx.is_empty()
        && idx.bytes().all(|b| b.is_ascii_digit())
    {
        let stem = &seg[..open];
        if !stem.is_empty()
            && stem
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            && stem.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Some((stem, idx));
        }
    }
    let digit_start = seg
        .bytes()
        .rposition(|b| !b.is_ascii_digit())
        .map_or(0, |i| i + 1);
    if digit_start > 0 && digit_start < seg.len() {
        let stem = &seg[..digit_start];
        let idx = &seg[digit_start..];
        if stem.bytes().all(|b| b.is_ascii_alphabetic() || b == b'_') {
            return Some((stem, idx));
        }
    }
    None
}

fn common_char_prefix<'a>(strs: &[&'a str]) -> &'a str {
    let Some((first, rest)) = strs.split_first() else {
        return "";
    };
    let mut len = first.len();
    for s in rest {
        len = first
            .char_indices()
            .zip(s.chars())
            .take_while(|&((i, a), b)| i < len && a == b)
            .last()
            .map_or(0, |((i, a), _)| i + a.len_utf8());
        if len == 0 {
            break;
        }
    }
    &first[..len]
}

fn common_char_suffix<'a>(strs: &[&'a str]) -> &'a str {
    let Some((first, rest)) = strs.split_first() else {
        return "";
    };
    let mut byte_len = first.len();
    for s in rest {
        let matched = first
            .chars()
            .rev()
            .zip(s.chars().rev())
            .take_while(|&(a, b)| a == b)
            .map(|(a, _)| a.len_utf8())
            .sum::<usize>();
        byte_len = byte_len.min(matched);
        if byte_len == 0 {
            break;
        }
    }
    &first[first.len() - byte_len..]
}

fn all_unique(items: &[String]) -> bool {
    let mut seen = HashSet::with_capacity(items.len());
    items.iter().all(|item| seen.insert(item.as_str()))
}

fn first_letter_badge(stem: &str, idx: &str) -> String {
    let letter = stem
        .chars()
        .find(char::is_ascii_alphabetic)
        .or_else(|| stem.chars().next())
        .unwrap_or('s')
        .to_ascii_lowercase();
    format!("{letter}{idx}")
}

fn is_token_delimiter(c: char) -> bool {
    matches!(c, '_' | '.' | '-' | '[' | ']' | '/')
}

fn snap_prefix_to_boundary<'a>(prefix: &'a str, strs: &[&str]) -> &'a str {
    let prefix = prefix.trim_end_matches(|c: char| c.is_ascii_digit());
    if prefix.is_empty() || prefix.ends_with(is_token_delimiter) {
        return prefix;
    }
    // Keep a non-delimiter prefix when every string continues directly with an index digit
    // (e.g. `lane0_a`, `lane1_a`).
    if strs
        .iter()
        .all(|s| s[prefix.len()..].starts_with(|c: char| c.is_ascii_digit()))
    {
        return prefix;
    }
    match prefix.rfind(is_token_delimiter) {
        Some(i) => &prefix[..=i],
        None => "",
    }
}

fn snap_suffix_to_boundary(suffix: &str) -> &str {
    let suffix = suffix.trim_start_matches(|c: char| c.is_ascii_digit());
    if suffix.is_empty() || suffix.starts_with(is_token_delimiter) {
        return suffix;
    }
    match suffix.find(is_token_delimiter) {
        Some(i) => &suffix[i..],
        None => "",
    }
}

fn try_trace_prefixed(sources: &[PipelineSource], candidates: &[String]) -> Option<Vec<String>> {
    let prefixed: Vec<String> = sources
        .iter()
        .zip(candidates)
        .map(|(s, badge)| {
            let full = format!("{}:{badge}", s.track.trace());
            full.chars().take(MAX_ALIAS_CHARS).collect()
        })
        .collect();
    all_unique(&prefixed).then_some(prefixed)
}

fn strip_bracket_index(seg: &str) -> &str {
    match seg.rfind('[') {
        Some(open) if seg.ends_with(']') => &seg[..open],
        _ => seg,
    }
}

/// Derive compact badges, chip labels, and default panel titles from arbitrary VTR paths.
#[must_use]
pub fn compute_source_badges(sources: &[PipelineSource]) -> SourceBadges {
    if sources.is_empty() {
        return SourceBadges {
            panel_title: "Empty pipeline".into(),
            compact_title: "Pipeline".into(),
            labels: Vec::new(),
            max_badge_chars: 2,
        };
    }

    let paths: Vec<&[String]> = sources.iter().map(|s| s.track.path()).collect();
    if sources.len() == 1 {
        let path = paths[0];
        let full = path.join(".");
        let compact = path.last().cloned().unwrap_or_default();
        let dist = if path.len() >= 2 {
            path[path.len() - 2..].join(".")
        } else {
            full.clone()
        };
        let auto_badge = path
            .iter()
            .rev()
            .find_map(|seg| {
                parse_indexed_segment(seg).map(|(stem, idx)| first_letter_badge(stem, idx))
            })
            .unwrap_or_else(|| {
                let fallback = path.last().map_or("s0", String::as_str);
                let taken: String = fallback.chars().take(4).collect();
                if taken.is_empty() { "s0".into() } else { taken }
            });
        let badge = clean_alias(sources[0].alias.as_deref()).unwrap_or_else(|| auto_badge.clone());
        let max_badge_chars = badge.chars().count().max(2);
        return SourceBadges {
            panel_title: full,
            compact_title: compact,
            labels: vec![SourceLabel {
                distinguishing: dist,
                auto_badge,
                badge,
            }],
            max_badge_chars,
        };
    }

    let min_len = paths.iter().map(|p| p.len()).min().unwrap_or(0);
    let mut prefix_len = 0;
    while prefix_len < min_len && paths.iter().all(|p| p[prefix_len] == paths[0][prefix_len]) {
        prefix_len += 1;
    }

    let mut suffix_len = 0;
    while suffix_len < min_len - prefix_len
        && paths
            .iter()
            .all(|p| p[p.len() - 1 - suffix_len] == paths[0][paths[0].len() - 1 - suffix_len])
    {
        suffix_len += 1;
    }

    let dist_lists: Vec<Vec<&str>> = paths
        .iter()
        .map(|p| {
            let slice = &p[prefix_len..p.len() - suffix_len];
            if slice.is_empty() {
                p.last().map_or_else(Vec::new, |s| vec![s.as_str()])
            } else {
                slice.iter().map(String::as_str).collect()
            }
        })
        .collect();

    // Panel title from shared prefix & suffix.
    let prefix_full = if prefix_len >= 2 {
        format!(
            "{} · {}",
            paths[0][prefix_len - 2],
            paths[0][prefix_len - 1]
        )
    } else if prefix_len == 1 {
        paths[0][0].clone()
    } else {
        String::new()
    };
    let prefix_compact = if prefix_len >= 1 {
        paths[0][prefix_len - 1].clone()
    } else {
        String::new()
    };
    let suffix_text = if suffix_len > 0 {
        paths[0][paths[0].len() - suffix_len..].join("/")
    } else {
        String::new()
    };
    let count = sources.len();
    let panel_title = match (prefix_full.is_empty(), suffix_text.is_empty()) {
        (false, false) => format!("{prefix_full}/{suffix_text} ({count})"),
        (false, true) => format!("{prefix_full} ({count})"),
        (true, false) => format!("{suffix_text} ({count})"),
        (true, true) => {
            let first = dist_lists[0]
                .first()
                .map_or("Pipeline", |s| strip_bracket_index(s));
            format!("{first} ({count})")
        }
    };
    let compact_title = match (prefix_compact.is_empty(), suffix_text.is_empty()) {
        (false, false) => format!("{prefix_compact}/{suffix_text} ({count})"),
        (false, true) => format!("{prefix_compact} ({count})"),
        (true, false) => format!("{suffix_text} ({count})"),
        (true, true) => format!("Merged ({count})"),
    };

    // Derive compact auto-badges preserving 0-based hardware indices.
    let multi_trace = sources
        .iter()
        .any(|s| s.track.trace() != sources[0].track.trace());
    let depth = dist_lists[0].len();
    let all_same_depth = depth > 0 && dist_lists.iter().all(|l| l.len() == depth);
    let mut auto_badges: Option<Vec<String>> = None;

    if all_same_depth && depth == 1 {
        let parsed: Option<Vec<(&str, &str)>> = dist_lists
            .iter()
            .map(|l| parse_indexed_segment(l[0]))
            .collect();
        if let Some(parsed) = parsed
            && parsed.iter().all(|(stem, _)| *stem == parsed[0].0)
        {
            let candidates: Vec<String> = parsed
                .iter()
                .map(|(stem, idx)| first_letter_badge(stem, idx))
                .collect();
            if all_unique(&candidates) {
                auto_badges = Some(candidates);
            } else if multi_trace {
                auto_badges = try_trace_prefixed(sources, &candidates);
            }
        }
    } else if all_same_depth && depth > 1 {
        let varying_cols: Vec<usize> = (0..depth)
            .filter(|&col| dist_lists.iter().any(|l| l[col] != dist_lists[0][col]))
            .collect();
        if !varying_cols.is_empty()
            && varying_cols.iter().all(|&col| {
                let Some((first_stem, _)) = parse_indexed_segment(dist_lists[0][col]) else {
                    return false;
                };
                dist_lists
                    .iter()
                    .all(|l| parse_indexed_segment(l[col]).is_some_and(|(s, _)| s == first_stem))
            })
        {
            let candidates: Vec<String> = dist_lists
                .iter()
                .map(|l| {
                    varying_cols
                        .iter()
                        .map(|&col| parse_indexed_segment(l[col]).unwrap().1)
                        .collect::<Vec<_>>()
                        .join(":")
                })
                .collect();
            if all_unique(&candidates) {
                auto_badges = Some(candidates);
            } else if multi_trace {
                auto_badges = try_trace_prefixed(sources, &candidates);
            }
        }
    }

    let auto_badges = auto_badges.unwrap_or_else(|| {
        let joined: Vec<String> = dist_lists.iter().map(|l| l.join(".")).collect();
        let all_identical = joined.iter().all(|s| s == &joined[0]);
        if all_identical {
            return (0..sources.len()).map(|i| format!("#{i}")).collect();
        }
        let refs: Vec<&str> = joined.iter().map(String::as_str).collect();
        let cp = snap_prefix_to_boundary(common_char_prefix(&refs), &refs);
        let cs = snap_suffix_to_boundary(common_char_suffix(&refs));
        let trimmed: Vec<String> = joined
            .iter()
            .map(|s| {
                let start = cp.len().min(s.len());
                let end = s.len().saturating_sub(cs.len()).max(start);
                let core =
                    s[start..end].trim_matches(|c: char| matches!(c, '_' | '.' | '-' | '[' | ']'));
                if core.is_empty() {
                    s.clone()
                } else {
                    core.to_owned()
                }
            })
            .collect();

        for width in [3, 4, 5, 6] {
            let candidates: Vec<String> = trimmed
                .iter()
                .map(|t| t.chars().take(width).collect())
                .collect();
            if candidates.iter().all(|c| !c.is_empty()) && all_unique(&candidates) {
                return candidates;
            }
        }
        if multi_trace {
            let base: Vec<String> = trimmed
                .iter()
                .map(|t| t.chars().take(3).collect())
                .collect();
            if base.iter().all(|c| !c.is_empty())
                && let Some(prefixed) = try_trace_prefixed(sources, &base)
            {
                return prefixed;
            }
        }
        (0..sources.len()).map(|i| format!("#{i}")).collect()
    });

    let mut labels = Vec::with_capacity(sources.len());
    let mut max_badge_chars = 2usize;
    for (i, source) in sources.iter().enumerate() {
        let base_dist = dist_lists[i].join(".");
        let distinguishing = if multi_trace {
            format!("{}: {base_dist}", source.track.trace())
        } else {
            base_dist
        };
        let auto_badge = auto_badges[i].clone();
        let badge = clean_alias(source.alias.as_deref()).unwrap_or_else(|| auto_badge.clone());
        max_badge_chars = max_badge_chars.max(badge.chars().count());
        labels.push(SourceLabel {
            distinguishing,
            auto_badge,
            badge,
        });
    }

    SourceBadges {
        panel_title,
        compact_title,
        labels,
        max_badge_chars,
    }
}

/// Detect when a merged panel mixes sources whose stage vocabularies share no stage names.
#[must_use]
pub fn disjoint_stage_notice(
    sources: &[PipelineSource],
    loaded: &[(usize, Vec<Arc<LoadedGenerator>>)],
) -> Option<String> {
    let mut vocabularies: Vec<(usize, HashSet<&str>)> = Vec::new();
    for (source_idx, gens) in loaded {
        let mut names = HashSet::new();
        for g in gens {
            for lane in &g.stage_census().lanes {
                for n in &lane.names {
                    names.insert(n.name.as_str());
                }
            }
        }
        if !names.is_empty() {
            vocabularies.push((*source_idx, names));
        }
    }
    if vocabularies.len() < 2 {
        return None;
    }
    let (first_idx, first_set) = &vocabularies[0];
    for (other_idx, other_set) in &vocabularies[1..] {
        if first_set.is_disjoint(other_set) {
            let a = sources[*first_idx]
                .track
                .path()
                .last()
                .map_or("source", String::as_str);
            let b = sources[*other_idx]
                .track
                .path()
                .last()
                .map_or("source", String::as_str);
            return Some(format!(
                "Sources mix disjoint stage vocabularies ({a} vs {b}). Separate synchronized pipeline panels are usually clearer for heterogeneous levels."
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::TraceId;

    fn src(trace: TraceId, path: &[&str], alias: Option<&str>) -> PipelineSource {
        PipelineSource {
            track: TrackSource::Unresolved {
                trace,
                path: path.iter().map(|s| (*s).to_owned()).collect(),
            },
            visible: true,
            alias: alias.map(str::to_owned),
            color_slot: 0,
        }
    }

    // User scenario: an engineer merges warp_slot[0..7]/Instruction from one SIMT partition
    // and expects compact 0-based badges (w0..w7) and a clean panel title.
    #[test]
    fn single_partition_warp_slots_produce_zero_based_w_badges_and_title() {
        let sources: Vec<_> = (0..8)
            .map(|slot| {
                let warp = format!("warp_slot[{slot}]");
                src(
                    TraceId::A,
                    &[
                        "gpu",
                        "compute_cluster[0]",
                        "sm[0]",
                        "simt_partition[0]",
                        &warp,
                        "Instruction",
                    ],
                    None,
                )
            })
            .collect();
        let badges = compute_source_badges(&sources);
        let actual: Vec<&str> = badges.labels.iter().map(|l| l.badge.as_str()).collect();
        assert_eq!(actual, ["w0", "w1", "w2", "w3", "w4", "w5", "w6", "w7"]);
        assert_eq!(badges.labels[0].distinguishing, "warp_slot[0]");
        assert_eq!(
            badges.panel_title,
            "sm[0] · simt_partition[0]/Instruction (8)"
        );
    }

    // User scenario: an engineer adds a warp slot from a second SIMT partition into the same
    // panel and expects badges to adapt automatically to colon-joined 0-based indices (0:0..1:7).
    #[test]
    fn multi_partition_warp_slots_adapt_to_colon_joined_indices() {
        let sources = vec![
            src(
                TraceId::A,
                &[
                    "gpu",
                    "sm[0]",
                    "simt_partition[0]",
                    "warp_slot[0]",
                    "Instruction",
                ],
                None,
            ),
            src(
                TraceId::A,
                &[
                    "gpu",
                    "sm[0]",
                    "simt_partition[0]",
                    "warp_slot[7]",
                    "Instruction",
                ],
                None,
            ),
            src(
                TraceId::A,
                &[
                    "gpu",
                    "sm[0]",
                    "simt_partition[1]",
                    "warp_slot[0]",
                    "Instruction",
                ],
                None,
            ),
            src(
                TraceId::A,
                &[
                    "gpu",
                    "sm[0]",
                    "simt_partition[1]",
                    "warp_slot[7]",
                    "Instruction",
                ],
                None,
            ),
        ];
        let badges = compute_source_badges(&sources);
        let actual: Vec<&str> = badges.labels.iter().map(|l| l.badge.as_str()).collect();
        assert_eq!(actual, ["0:0", "0:7", "1:0", "1:7"]);
        assert_eq!(badges.panel_title, "gpu · sm[0]/Instruction (4)");
    }

    // User scenario: an engineer merges arbitrary non-indexed VTR generators (e.g. LSU/DMA or
    // named NoC ports) and expects concise distinguishing prefixes.
    #[test]
    fn arbitrary_named_peers_derive_concise_distinguishing_badges() {
        let noc = vec![
            src(TraceId::A, &["soc", "noc", "port_north", "tx"], None),
            src(TraceId::A, &["soc", "noc", "port_south", "tx"], None),
        ];
        let noc_badges: Vec<_> = compute_source_badges(&noc)
            .labels
            .into_iter()
            .map(|l| l.badge)
            .collect();
        assert_eq!(noc_badges, ["nor", "sou"]);

        let units = vec![
            src(
                TraceId::A,
                &["sm[0]", "load_store_unit", "MemoryRequest"],
                None,
            ),
            src(TraceId::A, &["sm[0]", "dma_engine", "MemoryRequest"], None),
        ];
        let unit_badges: Vec<_> = compute_source_badges(&units)
            .labels
            .into_iter()
            .map(|l| l.badge)
            .collect();
        assert_eq!(unit_badges, ["loa", "dma"]);
    }

    // User scenario: an engineer compares the same VTR path across two traces or colliding names
    // and expects deterministic #0..#N fallback without duplicate badges; custom aliases always win.
    #[test]
    fn identical_paths_fallback_to_slot_hashes_and_custom_alias_overrides() {
        let same_path = vec![
            src(TraceId::A, &["sm[0]", "warp_slot[0]", "Instruction"], None),
            src(
                TraceId::new(1).unwrap(),
                &["sm[0]", "warp_slot[0]", "Instruction"],
                None,
            ),
        ];
        let badges = compute_source_badges(&same_path);
        let actual: Vec<&str> = badges.labels.iter().map(|l| l.badge.as_str()).collect();
        assert_eq!(actual, ["#0", "#1"]);

        let aliased = vec![
            src(
                TraceId::A,
                &["sm[0]", "warp_slot[0]", "Instruction"],
                Some("  BASE_LONG  "),
            ),
            src(
                TraceId::new(1).unwrap(),
                &["sm[0]", "warp_slot[0]", "Instruction"],
                Some("OPT"),
            ),
        ];
        let aliased_badges = compute_source_badges(&aliased);
        assert_eq!(aliased_badges.labels[0].badge, "BASE_L");
        assert_eq!(aliased_badges.labels[1].badge, "OPT");
        assert_eq!(aliased_badges.labels[0].auto_badge, "#0");
    }

    // User scenario: an engineer merges peers whose names share leading or trailing letters inside
    // a word (e.g. core[0]/cache[0], read_unit/load_unit) or compares multiple slots across two
    // traces, and expects legible word prefixes and trace-prefixed badges instead of clipped words.
    #[test]
    fn boundary_snapped_prefixes_and_multi_trace_slot_badges_stay_legible() {
        let same_letter = vec![
            src(TraceId::A, &["soc", "core[0]", "tx"], None),
            src(TraceId::A, &["soc", "cache[0]", "tx"], None),
        ];
        let same_letter_badges: Vec<_> = compute_source_badges(&same_letter)
            .labels
            .into_iter()
            .map(|l| l.badge)
            .collect();
        assert_eq!(same_letter_badges, ["cor", "cac"]);

        let shared_suffix = vec![
            src(TraceId::A, &["soc", "read_unit", "tx"], None),
            src(TraceId::A, &["soc", "load_unit", "tx"], None),
        ];
        let suffix_badges: Vec<_> = compute_source_badges(&shared_suffix)
            .labels
            .into_iter()
            .map(|l| l.badge)
            .collect();
        assert_eq!(suffix_badges, ["rea", "loa"]);

        let cross_trace = vec![
            src(TraceId::A, &["sm[0]", "warp_slot[0]", "Instruction"], None),
            src(TraceId::A, &["sm[0]", "warp_slot[1]", "Instruction"], None),
            src(
                TraceId::new(1).unwrap(),
                &["sm[0]", "warp_slot[0]", "Instruction"],
                None,
            ),
            src(
                TraceId::new(1).unwrap(),
                &["sm[0]", "warp_slot[1]", "Instruction"],
                None,
            ),
        ];
        let cross_badges: Vec<_> = compute_source_badges(&cross_trace)
            .labels
            .into_iter()
            .map(|l| l.badge)
            .collect();
        assert_eq!(cross_badges, ["A:w0", "A:w1", "B:w0", "B:w1"]);
    }
}
