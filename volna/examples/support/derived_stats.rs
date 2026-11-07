//! Five interval counters and their provenance, kept independently of the large source recording.

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::Path;
use vtr::{NodeId, Reader, Recorder, SignalId, SignalValue, Value, Writer};

const SOURCE: &str = "workspace/output/runs/hip-matmul-lds-gem5-kmd-vtr/trace.vtr";
const COUNTERS: [(&str, &str); 5] = [
    ("lds_read_bytes", "rd_bytes"),
    ("lds_write_bytes", "wr_bytes"),
    ("lds_conflict_cycles", "conflict_cycles"),
    ("lds_bank_accesses", "bank_accesses"),
    ("lds_requests", "requests"),
];

#[derive(Deserialize, Serialize)]
pub struct Counter {
    pub name: String,
    pub unit: String,
    pub description: String,
    pub source_path: String,
}

#[derive(Deserialize, Serialize)]
pub struct Samples {
    pub source: String,
    pub description: String,
    pub recording_id: String,
    pub start_gpu_cycle: u64,
    pub clock_period_ps: u64,
    pub counters: [Counter; 5],
    pub rows: Vec<[u64; 5]>,
}

impl Samples {
    pub fn bundled() -> Result<Self> {
        Self::read(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/support/derived_stats.json"),
        )
    }

    pub fn read(path: &Path) -> Result<Self> {
        let samples: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        samples.validate()?;
        Ok(samples)
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            (16..=64).contains(&self.rows.len()),
            "use 16–64 complete statistics intervals"
        );
        ensure!(
            self.clock_period_ps == super::CLOCK_PS,
            "statistics and playground clocks differ"
        );
        for (index, counter) in self.counters.iter().enumerate() {
            ensure!(
                !counter.name.is_empty() && !counter.source_path.is_empty(),
                "counter needs a name and source path"
            );
            ensure!(
                self.counters[..index]
                    .iter()
                    .all(|c| c.name != counter.name),
                "duplicate counter name"
            );
            ensure!(
                self.rows
                    .iter()
                    .all(|row| u32::try_from(row[index]).is_ok()),
                "counter exceeds 32 bits"
            );
            ensure!(
                self.rows.iter().any(|row| row[index] > 0),
                "counter {} has no activity",
                counter.name
            );
        }
        Ok(())
    }

    pub fn extract(source: &Path, output: &Path, start_gpu_cycle: u64) -> Result<()> {
        let reader = Reader::open(source)?;
        ensure!(
            reader.metadata().timescale == -12,
            "source must use picosecond ticks"
        );
        let clock = reader
            .find_generator("root.system.Shader.clk_domain.clock")
            .context("missing shader clock")?;
        let find = |path: &str| {
            reader
                .find_signal(path)
                .with_context(|| format!("missing {path}"))
        };
        let start = find("Stats.gpu0.SamplingWindow.start_gpu_clk")?;
        let duration = find("Stats.gpu0.SamplingWindow.duration_gpu_cycles")?;
        let flags = find("Stats.gpu0.SamplingWindow.status_flags")?;
        let generation = find("Stats.gpu0.SamplingWindow.stats_reset_generation")?;
        let bits = |id, time| {
            reader
                .value_at(id, time)?
                .borrow()
                .as_u64()
                .context("unknown sampling metadata")
        };
        let attribute = |path: &str, key: &str| -> Result<String> {
            let node = reader.node(reader.find(path).context("missing counter node")?)?;
            for attr in &node.attributes {
                if reader.string(attr.key)? == key {
                    return match &attr.value {
                        Value::Text(text) => Ok(text.clone()),
                        Value::Str(id) => Ok(reader.string(*id)?.into()),
                        _ => anyhow::bail!("counter {key} is not text"),
                    };
                }
            }
            anyhow::bail!("counter lacks {key}")
        };
        let counters = COUNTERS
            .map(|(name, source_name)| -> Result<Counter> {
                let path = format!("Stats.gpu0.Summary.LDS.{source_name}");
                Ok(Counter {
                    name: name.into(),
                    unit: attribute(&path, "units")?,
                    description: attribute(&path, "vtr.description")?,
                    source_path: path,
                })
            })
            .into_iter()
            .collect::<Result<Vec<_>>>()?;
        let signals = counters
            .iter()
            .map(|c| find(&c.source_path))
            .collect::<Result<Vec<_>>>()?;
        let mut rows = Vec::new();
        let mut reset_generation = None;
        for index in 0..64 {
            let first = start_gpu_cycle
                .checked_add(index * super::SAMPLE_CLOCKS)
                .context("cycle overflow")?;
            let end = first
                .checked_add(super::SAMPLE_CLOCKS)
                .context("cycle overflow")?;
            let time = end
                .checked_mul(super::CLOCK_PS)
                .context("timestamp overflow")?;
            ensure!(
                reader.cycle_at(clock, time)? == Some(end)
                    && reader.cycle_at(clock, time - 1)? == Some(end - 1),
                "source clock period or phase differs"
            );
            ensure!(
                bits(start, time)? == first
                    && bits(duration, time)? == super::SAMPLE_CLOCKS
                    && bits(flags, time)? == 2,
                "source interval ending at {time} is not a complete periodic 100-clock sample"
            );
            let current_generation = bits(generation, time)?;
            ensure!(
                reset_generation.is_none_or(|v| v == current_generation),
                "statistics reset in excerpt"
            );
            reset_generation = Some(current_generation);
            let mut row = [0; 5];
            for (value, &id) in row.iter_mut().zip(&signals) {
                let raw = reader.value_at(id, time)?;
                let SignalValue::Real(number) = raw.borrow() else {
                    anyhow::bail!("source interval counter is not real");
                };
                ensure!(
                    number.is_finite()
                        && number >= 0.0
                        && number <= f64::from(u32::MAX)
                        && number.fract() == 0.0,
                    "source counter is not an exact nonnegative 32-bit integer"
                );
                *value = number as u64;
            }
            rows.push(row);
        }
        let snapshot = Self {
            source: SOURCE.into(),
            description: format!(
                "64 consecutive complete 100-clock windows; source GPU cycles {start_gpu_cycle}..{}; values copied unchanged, then rebased to zero with a zero-count baseline.",
                start_gpu_cycle + 6400
            ),
            recording_id: reader.identity().to_string(),
            start_gpu_cycle,
            clock_period_ps: super::CLOCK_PS,
            counters: counters
                .try_into()
                .map_err(|_| anyhow::anyhow!("expected five counters"))?,
            rows,
        };
        snapshot.validate()?;
        std::fs::write(output, serde_json::to_vec_pretty(&snapshot)?)?;
        println!(
            "{}: 64 complete source intervals from recording {}",
            output.display(),
            snapshot.recording_id
        );
        Ok(())
    }

    pub fn declare(&self, writer: &mut Writer, scope: NodeId) -> Result<[SignalId; 5]> {
        let mut signals = [SignalId(0); 5];
        for (id, counter) in signals.iter_mut().zip(&self.counters) {
            *id = super::signal(
                writer,
                scope,
                &counter.name,
                32,
                &counter.unit,
                &format!(
                    "{}; interval count for the preceding 100 clocks; source: {}",
                    counter.description, counter.source_path
                ),
            )?;
        }
        Ok(signals)
    }

    pub fn emit_baseline(writer: &mut Writer, signals: &[SignalId; 5]) -> Result<()> {
        for &id in signals {
            writer.emit_u64(id, 0)?;
        }
        Ok(())
    }

    pub fn emit(&self, writer: &mut Writer, signals: &[SignalId; 5], index: usize) -> Result<()> {
        let values = self
            .rows
            .get(index)
            .context("missing statistics interval")?;
        for (&id, &value) in signals.iter().zip(values) {
            writer.emit_u64(id, value)?;
        }
        Ok(())
    }
}
