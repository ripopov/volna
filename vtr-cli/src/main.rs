use anyhow::{bail, ensure, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};
use vtr::{Reader, Writer};
mod inspect;

#[derive(Parser)]
#[command(
    name = "vtr",
    version,
    about = "Inspect, query, and convert hardware traces",
    after_help = "Times are unsigned integer ticks in the recording's time base. Paths use dots as separators. Exit codes: 0 success, 1 missing index, 2 error, 3 recovery dropped bytes."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Args)]
struct Window {
    /// First included tick
    #[arg(long, default_value_t = 0)]
    from: u64,
    /// Last included tick
    #[arg(long, default_value_t = u64::MAX)]
    to: u64,
}
impl Window {
    fn checked(&self) -> Result<(u64, u64)> {
        ensure!(self.from <= self.to, "--from must not exceed --to");
        Ok((self.from, self.to))
    }
}
#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Kanata,
    Vcd,
    Fst,
}
#[derive(Subcommand)]
enum Command {
    /// Show VTR metadata, counts, and sections
    Info { file: PathBuf },
    /// Show the VTR hierarchy
    Hierarchy {
        file: PathBuf,
        #[arg(long, default_value_t = usize::MAX)]
        depth: usize,
        /// Include variable declarations
        #[arg(long)]
        vars: bool,
        /// Show scope, variable, and distinct signal counts
        #[arg(long)]
        sizes: bool,
    },
    /// Read one VTR signal at a tick
    Value {
        file: PathBuf,
        signal: String,
        time: u64,
    },
    /// List changes of a VTR signal in an inclusive window
    Changes {
        file: PathBuf,
        signal: String,
        #[command(flatten)]
        window: Window,
        #[arg(long, default_value_t = 100)]
        max: usize,
    },
    /// Show VTR transactions, attributes, stages, and events
    Transactions {
        file: PathBuf,
        /// Full stream path
        #[arg(long, conflicts_with = "id")]
        stream: Option<String>,
        #[command(flatten)]
        window: Window,
        #[arg(long, default_value_t = 100)]
        max: usize,
        /// Select transaction IDs and include their incident relations
        #[arg(long, value_delimiter = ',', conflicts_with_all = ["from", "to", "max"])]
        id: Vec<u64>,
    },
    /// Show formatted VTR log messages or their call sites
    Logs {
        file: PathBuf,
        #[arg(long)]
        stream: Option<String>,
        #[command(flatten)]
        window: Window,
        #[arg(long, default_value_t = 100)]
        max: usize,
        #[arg(long, default_value = "trace", value_parser = parse_severity)]
        severity: vtr::Severity,
        #[arg(long, conflicts_with_all = ["stream", "from", "to", "max", "severity"])]
        sites: bool,
    },
    /// Show VTR declared clocks and edge stretches
    Clocks { file: PathBuf },
    /// Convert Kanata, VCD, or FST to a new VTR file
    Convert {
        input: PathBuf,
        output: PathBuf,
        /// Override format inferred from .kanata/.log[.gz], .vcd, or .fst
        #[arg(long, value_enum)]
        format: Option<Format>,
    },
    /// Export VTR waveforms as a new VCD file
    ToVcd { input: PathBuf, output: PathBuf },
    /// Recover complete VTR sections into a new file
    Recover { input: PathBuf, output: PathBuf },
    /// Build or check a VTR/FST activity sidecar
    Index {
        file: PathBuf,
        #[arg(long, conflicts_with_all = ["threads", "memory"])]
        check: bool,
        /// Worker count (0 uses the library default)
        #[arg(long, default_value_t = 0)]
        threads: usize,
        /// Builder scratch limit in MiB
        #[arg(long, default_value_t = 256)]
        memory: u64,
    },
    /// List distinct VTR/FST signals changing in an inclusive tick window
    Active {
        file: PathBuf,
        #[command(flatten)]
        window: Window,
        /// Build the sidecar if missing
        #[arg(long)]
        build: bool,
    },
}
fn parse_severity(s: &str) -> Result<vtr::Severity, String> {
    vtr::Severity::from_name(s)
        .ok_or_else(|| "expected trace, debug, info, warn, error, or fatal".into())
}

/// Publish only finished output; existing destinations (including symlinks) are refused.
fn output_file<T>(output: &Path, write: impl FnOnce(&Path) -> Result<T>) -> Result<T> {
    ensure!(
        !output.try_exists()? && std::fs::symlink_metadata(output).is_err(),
        "output {} already exists; choose a new path",
        output.display()
    );
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temp = tempfile::NamedTempFile::new_in(parent)?;
    let result = write(temp.path())?;
    temp.persist_noclobber(output)
        .with_context(|| format!("cannot publish {}", output.display()))?;
    Ok(result)
}
fn format_of(path: &Path) -> Result<Format> {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    let name = name.strip_suffix(".gz").unwrap_or(&name);
    if name.ends_with(".kanata") || name.ends_with(".log") {
        Ok(Format::Kanata)
    } else if name.ends_with(".vcd") {
        Ok(Format::Vcd)
    } else if name.ends_with(".fst") {
        Ok(Format::Fst)
    } else {
        bail!("unknown input format; use --format kanata, vcd, or fst")
    }
}
fn open_session(path: PathBuf) -> Result<std::sync::Arc<dyn volna_trace::Session>> {
    volna_trace::OpenSpec::Path(path.clone())
        .open()
        .with_context(|| format!("cannot open {}", path.display()))
}
fn build_index(session: &dyn volna_trace::Session, threads: usize, memory: u64) -> Result<()> {
    let memory = memory
        .checked_mul(1 << 20)
        .context("--memory is too large")?;
    ensure!(memory > 0, "--memory must be positive");
    let options = vtr::activity::BuildOptions {
        threads,
        memory,
        control: Some(std::sync::Arc::new(vtr::activity::BuildControl::default())),
        ..Default::default()
    };
    session.build_activity(
        &options,
        &volna_trace::remote::memory::MemoryBudget::new(u64::MAX),
        vtr::activity::default_cache_dir().as_deref(),
    )
}
fn run(cli: Cli) -> Result<u8> {
    match cli.command {
        Command::Info { file } => inspect::info(&file)?,
        Command::Hierarchy {
            file,
            depth,
            vars,
            sizes,
        } => inspect::hierarchy(&file, depth, vars, sizes)?,
        Command::Value { file, signal, time } => {
            let r = Reader::open(file)?;
            let id = r
                .find_signal(&signal, '.')
                .with_context(|| format!("signal {signal:?} not found"))?;
            println!("{}", r.value_at(id, time)?.to_ascii());
        }
        Command::Changes {
            file,
            signal,
            window,
            max,
        } => {
            let (from, to) = window.checked()?;
            let r = Reader::open(file)?;
            let id = r
                .find_signal(&signal, '.')
                .with_context(|| format!("signal {signal:?} not found"))?;
            let changes = r.changes(id, from, to)?;
            for (t, value) in changes.iter().take(max) {
                println!("{t}\t{}", value.to_ascii());
            }
            if changes.len() > max {
                eprintln!(
                    "{} more changes; increase --max to show them",
                    changes.len() - max
                );
            }
        }
        Command::Transactions {
            file,
            stream,
            window,
            max,
            id,
        } => inspect::transactions(&file, stream.as_deref(), window.checked()?, max, &id)?,
        Command::Logs {
            file,
            stream,
            window,
            max,
            severity,
            sites,
        } => inspect::logs(
            &file,
            stream.as_deref(),
            window.checked()?,
            max,
            severity,
            sites,
        )?,
        Command::Clocks { file } => inspect::clocks(&file)?,
        Command::Convert {
            input,
            output,
            format,
        } => {
            let format = format.map(Ok).unwrap_or_else(|| format_of(&input))?;
            output_file(&output, |path| {
                let mut writer = Writer::create_with(
                    path,
                    vtr::WriterOptions {
                        dedup: false,
                        ..Default::default()
                    },
                )?;
                match format {
                    Format::Kanata => vtr_cli::kanata::convert_kanata(&input, &mut writer),
                    Format::Vcd => vtr_cli::vcd::convert_vcd(&input, &mut writer),
                    Format::Fst => vtr_cli::fst::convert_fst(&input, &mut writer),
                }
                .with_context(|| format!("converting {}", input.display()))?;
                writer.close()?;
                Ok(())
            })?;
            println!("wrote {}", output.display());
        }
        Command::ToVcd { input, output } => {
            let reader = Reader::open(input)?;
            let changes = output_file(&output, |path| {
                vtr_cli::vcdout::write_vcd(
                    &reader,
                    std::io::BufWriter::new(std::fs::File::create(path)?),
                )
            })?;
            println!("wrote {} changes to {}", changes, output.display());
        }
        Command::Recover { input, output } => {
            let dropped = output_file(&output, |path| Ok(vtr::recover(&input, path)?))?;
            println!(
                "wrote {}; dropped {} bytes",
                output.display(),
                dropped.unwrap_or(0)
            );
            if dropped.is_some_and(|n| n > 0) {
                return Ok(3);
            }
        }
        Command::Index {
            file,
            check,
            threads,
            memory,
        } => {
            let session = open_session(file)?;
            if check && session.activity().is_none() {
                println!("no valid activity index; run vtr index <file>");
                return Ok(1);
            }
            if !check {
                build_index(session.as_ref(), threads, memory)?;
            }
            println!(
                "valid activity index: {} blocks",
                session
                    .activity()
                    .context("index not available")?
                    .blocks()
                    .len()
            );
        }
        Command::Active {
            file,
            window,
            build,
        } => {
            let (from, to) = window.checked()?;
            let session = open_session(file)?;
            if build {
                build_index(session.as_ref(), 0, 256)?;
            }
            let index = session
                .activity()
                .context("no valid activity index; run vtr index <file> or pass --build")?;
            let classification = index.classify(from, to);
            let undecided: Vec<_> = classification
                .undecided
                .iter()
                .map(|s| volna_trace::data::SignalRef(s.0))
                .collect();
            let mut active: std::collections::BTreeSet<_> = classification
                .active
                .iter()
                .map(|s| volna_trace::data::SignalRef(s.0))
                .collect();
            active.extend(session.resolve_activity(&undecided, from, to)?);
            for (id, var) in session.hierarchy().vars().enumerate() {
                if active.remove(&var.signal) {
                    println!("{}", session.hierarchy().full_name(id));
                }
            }
        }
    }
    Ok(0)
}
fn main() -> std::process::ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code.into(),
        Err(error) => {
            eprintln!("error: {error:#}");
            2.into()
        }
    }
}
