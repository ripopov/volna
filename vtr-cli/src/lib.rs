//! Hardware trace conversion. VTR/FST reading and activity indexing use
//! [`vtr`] and [`volna_trace`]. Converters accept a fresh [`vtr::Writer`]; callers
//! select writer options, close on success, and discard output after errors.
//!
//! ```
//! use vtr::{Writer, WriterOptions};
//! let dir = tempfile::tempdir()?;
//! let input = dir.path().join("pipeline.kanata");
//! std::fs::write(&input, "Kanata\t0004\nC=\t100\nI\t0\t0\t0\nC\t1\nR\t0\t0\t0\n")?;
//! let mut writer = Writer::create_with(dir.path().join("pipeline.vtr"),
//!     WriterOptions { dedup: false, ..Default::default() })?;
//! vtr_cli::kanata::convert_kanata(input, &mut writer)?;
//! writer.close()?;
//! # Ok::<(), anyhow::Error>(())
//! ```
pub mod fst;
pub mod kanata;
pub mod vcd;
pub mod vcdout;
