//! Recording access shared by local and remote loaders.

use std::sync::Arc;

use crate::data::{Hierarchy, SignalHistory, SignalRef, TraceInfo};

pub use crate::data::vtr_source::LocalSession;

pub type SignalLoads = Vec<(SignalRef, anyhow::Result<Arc<dyn SignalHistory>>)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Capabilities {
    pub waveforms: bool,
    pub transactions: bool,
    pub relations: bool,
}

pub trait Session: Send + Sync {
    fn resident_bytes(&self) -> u64 {
        0
    }

    fn memory_budget(&self) -> Option<crate::remote::memory::MemoryBudget> {
        None
    }

    fn format(&self) -> Option<&'static str> {
        None
    }

    fn remote_id(&self) -> Option<u64> {
        None
    }
    fn tracks(&self) -> &[crate::data::transactions::Track] {
        &[]
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            waveforms: true,
            transactions: false,
            relations: false,
        }
    }
    fn info(&self) -> &TraceInfo;
    fn hierarchy(&self) -> &Hierarchy;
    fn scope_sizes(&self) -> Option<Arc<crate::data::ScopeSizes>> {
        None
    }
    fn activity(&self) -> Option<Arc<crate::data::ActivityIndex>> {
        None
    }
    fn activity_build_info(&self) -> Option<crate::data::ActivityBuildInfo> {
        None
    }
    fn activity_identity(&self) -> Option<vtr::activity::Identity> {
        None
    }
    fn activity_image(&self, _cache_dir: Option<&std::path::Path>) -> anyhow::Result<Vec<u8>> {
        anyhow::bail!("this session has no activity sidecar")
    }
    fn activity_host(&self) -> Option<&str> {
        None
    }
    fn activity_available(&self) -> bool {
        self.activity().is_some()
    }
    fn install_activity(&self, _index: Arc<crate::data::ActivityIndex>) -> anyhow::Result<()> {
        anyhow::bail!("this session does not accept remote activity indexes")
    }
    fn build_activity(
        &self,
        _options: &vtr::activity::BuildOptions,
        _budget: &crate::remote::memory::MemoryBudget,
        _cache_dir: Option<&std::path::Path>,
    ) -> anyhow::Result<()> {
        anyhow::bail!("this session cannot build an activity index")
    }
    fn resolve_activity(
        &self,
        _signals: &[SignalRef],
        _t0: u64,
        _t1: u64,
    ) -> anyhow::Result<Vec<SignalRef>> {
        anyhow::bail!("this trace cannot read activity")
    }
    fn load_track(
        &self,
        _track: crate::data::transactions::TrackRef,
    ) -> anyhow::Result<crate::data::loaded_tracks::LoadedTrack> {
        anyhow::bail!("transaction track loading is unsupported")
    }
    fn load_signal(&self, signal: SignalRef) -> anyhow::Result<Arc<dyn SignalHistory>>;

    fn load_signals(&self, signals: &[SignalRef]) -> SignalLoads {
        let mut loaded = std::collections::BTreeMap::new();
        signals
            .iter()
            .map(|&signal| {
                let result = loaded
                    .entry(signal)
                    .or_insert_with(|| self.load_signal(signal));
                (
                    signal,
                    match result {
                        Ok(history) => Ok(Arc::clone(history)),
                        Err(error) => Err(anyhow::anyhow!("{error:#}")),
                    },
                )
            })
            .collect()
    }
}

pub fn account_local_session(
    session: Arc<dyn Session>,
    budget: crate::remote::memory::MemoryBudget,
) -> anyhow::Result<Arc<dyn Session>> {
    if session.memory_budget().is_some() {
        return Ok(session);
    }
    let resident_bytes = session.resident_bytes();
    if resident_bytes == 0 {
        return Ok(session);
    }
    let reservation = budget.reserve(resident_bytes)?;
    Ok(Arc::new(AccountedSession {
        inner: session,
        budget,
        _reservation: reservation,
    }))
}

struct AccountedSession {
    inner: Arc<dyn Session>,
    budget: crate::remote::memory::MemoryBudget,
    _reservation: crate::remote::memory::Reservation,
}

impl Session for AccountedSession {
    fn resident_bytes(&self) -> u64 {
        self.inner.resident_bytes()
    }
    fn memory_budget(&self) -> Option<crate::remote::memory::MemoryBudget> {
        Some(self.budget.clone())
    }
    fn format(&self) -> Option<&'static str> {
        self.inner.format()
    }
    fn tracks(&self) -> &[crate::data::transactions::Track] {
        self.inner.tracks()
    }
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    fn info(&self) -> &TraceInfo {
        self.inner.info()
    }
    fn hierarchy(&self) -> &Hierarchy {
        self.inner.hierarchy()
    }
    fn scope_sizes(&self) -> Option<Arc<crate::data::ScopeSizes>> {
        self.inner.scope_sizes()
    }
    fn activity(&self) -> Option<Arc<crate::data::ActivityIndex>> {
        self.inner.activity()
    }
    fn activity_build_info(&self) -> Option<crate::data::ActivityBuildInfo> {
        self.inner.activity_build_info()
    }
    fn activity_identity(&self) -> Option<vtr::activity::Identity> {
        self.inner.activity_identity()
    }
    fn activity_image(&self, cache_dir: Option<&std::path::Path>) -> anyhow::Result<Vec<u8>> {
        self.inner.activity_image(cache_dir)
    }
    fn build_activity(
        &self,
        options: &vtr::activity::BuildOptions,
        budget: &crate::remote::memory::MemoryBudget,
        cache_dir: Option<&std::path::Path>,
    ) -> anyhow::Result<()> {
        self.inner.build_activity(options, budget, cache_dir)
    }
    fn resolve_activity(
        &self,
        signals: &[SignalRef],
        t0: u64,
        t1: u64,
    ) -> anyhow::Result<Vec<SignalRef>> {
        self.inner.resolve_activity(signals, t0, t1)
    }
    fn load_track(
        &self,
        track: crate::data::transactions::TrackRef,
    ) -> anyhow::Result<crate::data::loaded_tracks::LoadedTrack> {
        let mut loaded = self.inner.load_track(track)?;
        for generator in &mut loaded.generators {
            let Some(generator) = Arc::get_mut(generator) else {
                anyhow::bail!("new local track unexpectedly shared its generator owner");
            };
            if generator.reservation.is_none() {
                generator.reservation = Some(
                    self.budget
                        .reserve_object("a generator", generator.resident_bytes())?,
                );
            }
        }
        Ok(loaded)
    }
    fn load_signal(&self, signal: SignalRef) -> anyhow::Result<Arc<dyn SignalHistory>> {
        let history = self.inner.load_signal(signal)?;
        account_history(history, &self.budget)
    }
    fn load_signals(&self, signals: &[SignalRef]) -> SignalLoads {
        let mut accounted = std::collections::BTreeMap::new();
        self.inner
            .load_signals(signals)
            .into_iter()
            .map(|(signal, result)| {
                let result = match accounted.get(&signal) {
                    Some(history) => Ok(Arc::clone(history)),
                    None => result
                        .and_then(|history| account_history(history, &self.budget))
                        .inspect(|history| {
                            accounted.insert(signal, Arc::clone(history));
                        }),
                };
                (signal, result)
            })
            .collect()
    }
}

fn account_history(
    inner: Arc<dyn SignalHistory>,
    budget: &crate::remote::memory::MemoryBudget,
) -> anyhow::Result<Arc<dyn SignalHistory>> {
    let reservation = budget.reserve_object("the signal history", inner.resident_bytes())?;
    Ok(Arc::new(AccountedHistory {
        inner,
        _reservation: reservation,
    }))
}

struct AccountedHistory {
    inner: Arc<dyn SignalHistory>,
    _reservation: crate::remote::memory::Reservation,
}

impl SignalHistory for AccountedHistory {
    fn resident_bytes(&self) -> u64 {
        self.inner.resident_bytes()
    }
    fn shape(&self) -> crate::data::SignalShape {
        self.inner.shape()
    }
    fn len(&self) -> usize {
        self.inner.len()
    }
    fn time(&self, index: usize) -> u64 {
        self.inner.time(index)
    }
    fn value(&self, index: Option<usize>) -> crate::data::WaveValue {
        self.inner.value(index)
    }
    fn value_view(&self, index: Option<usize>) -> crate::data::value_view::ValueView<'_> {
        self.inner.value_view(index)
    }
    fn always_normal(&self) -> bool {
        self.inner.always_normal()
    }
    fn bit(&self, index: Option<usize>) -> crate::data::Bit {
        self.inner.bit(index)
    }
    fn index_at(&self, time: u64) -> Option<usize> {
        self.inner.index_at(time)
    }
    fn index_at_hint(&self, time: u64, hint: usize) -> Option<usize> {
        self.inner.index_at_hint(time, hint)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenSpec {
    Remote {
        name: String,
        limits: crate::remote::limits::Limits,
    },
    #[cfg(not(target_family = "wasm"))]
    Path(std::path::PathBuf),
    Bytes {
        name: String,
        bytes: Vec<u8>,
    },
}

impl OpenSpec {
    #[must_use]
    pub fn name(&self) -> String {
        match self {
            OpenSpec::Remote { name, .. } => name.clone(),
            #[cfg(not(target_family = "wasm"))]
            OpenSpec::Path(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            OpenSpec::Bytes { name, .. } => name.clone(),
        }
    }

    #[must_use]
    pub fn source(&self) -> String {
        match self {
            OpenSpec::Remote { name, .. } | OpenSpec::Bytes { name, .. } => name.clone(),
            #[cfg(not(target_family = "wasm"))]
            OpenSpec::Path(p) => p.to_string_lossy().into_owned(),
        }
    }

    pub fn open(self) -> anyhow::Result<Arc<dyn Session>> {
        match self {
            OpenSpec::Remote { .. } => {
                anyhow::bail!("remote opening requires the asynchronous executor")
            }
            #[cfg(not(target_family = "wasm"))]
            OpenSpec::Path(p) => {
                use std::io::{Read, Seek};
                let mut input = std::io::BufReader::new(std::fs::File::open(&p)?);
                let mut prefix = [0; 9];
                input.read_exact(&mut prefix)?;
                input.rewind()?;
                if is_fst(&prefix)? {
                    let name = p
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    let mut session =
                        crate::data::fst_source::FstSession::open(name, Box::new(input))?;
                    session.find_activity(&p);
                    Ok(Arc::new(session))
                } else {
                    Ok(Arc::new(LocalSession::open(&p)?))
                }
            }
            OpenSpec::Bytes { name, bytes } => {
                if is_fst(&bytes)? {
                    Ok(Arc::new(crate::data::fst_source::FstSession::open(
                        name,
                        Box::new(std::io::Cursor::new(bytes)),
                    )?))
                } else {
                    Ok(Arc::new(LocalSession::from_bytes(name, bytes)?))
                }
            }
        }
    }
}

fn is_fst(bytes: &[u8]) -> anyhow::Result<bool> {
    if bytes.starts_with(&vtr::container::FILE_MAGIC) {
        return Ok(false);
    }
    if bytes.len() >= 9
        && (bytes[0] == 254
            || (bytes[0] == 0 && u64::from_be_bytes(bytes[1..9].try_into().unwrap()) == 329))
    {
        return Ok(true);
    }
    anyhow::bail!("unrecognized trace format: expected VTR or FST header")
}

pub enum LoadRequest<Tag = u64> {
    BuildActivity {
        tag: Tag,
        generation: u64,
        session: Arc<dyn Session>,
        options: vtr::activity::BuildOptions,
        cache_dir: Option<std::path::PathBuf>,
        budget: crate::remote::memory::MemoryBudget,
    },
    Track {
        tag: Tag,
        generation: u64,
        request_id: u64,
        session: Arc<dyn Session>,
        track: crate::data::transactions::TrackRef,
    },
    Signals {
        tag: Tag,
        generation: u64,
        session: Arc<dyn Session>,
        signals: Vec<SignalRef>,
    },
    ResolveActivity {
        tag: Tag,
        generation: u64,
        session: Arc<dyn Session>,
        signals: Vec<SignalRef>,
        window: (u64, u64),
        budget: crate::remote::memory::MemoryBudget,
    },
}

impl<Tag: Copy> LoadRequest<Tag> {
    pub fn remote_id(&self) -> Option<u64> {
        match self {
            Self::Signals { session, .. }
            | Self::Track { session, .. }
            | Self::BuildActivity { session, .. }
            | Self::ResolveActivity { session, .. } => session.remote_id(),
        }
    }
    pub fn fail(self, error: anyhow::Error) -> LoadResult<Tag> {
        match self {
            Self::BuildActivity {
                tag, generation, ..
            } => LoadResult::ActivityBuilt {
                tag,
                generation,
                result: Err(error),
            },
            Self::Track {
                tag,
                generation,
                request_id,
                track,
                ..
            } => LoadResult::Track {
                tag,
                generation,
                request_id,
                track,
                result: Err(error),
            },
            Self::Signals {
                tag,
                generation,
                signals,
                ..
            } => LoadResult::Signals {
                tag,
                generation,
                results: signals
                    .into_iter()
                    .map(|id| (id, Err(anyhow::anyhow!("{error:#}"))))
                    .collect(),
            },
            Self::ResolveActivity {
                tag,
                generation,
                window,
                ..
            } => LoadResult::ActivityResolved {
                tag,
                generation,
                window,
                result: Err(error),
            },
        }
    }
    pub fn perform(self) -> LoadResult<Tag> {
        match self {
            Self::BuildActivity {
                tag,
                generation,
                session,
                options,
                cache_dir,
                budget,
            } => LoadResult::ActivityBuilt {
                tag,
                generation,
                result: session.build_activity(&options, &budget, cache_dir.as_deref()),
            },
            Self::Track {
                tag,
                generation,
                request_id,
                session,
                track,
            } => LoadResult::Track {
                tag,
                generation,
                request_id,
                track,
                result: session.load_track(track),
            },
            Self::Signals {
                tag,
                generation,
                session,
                signals,
            } => LoadResult::Signals {
                tag,
                generation,
                results: session.load_signals(&signals),
            },
            Self::ResolveActivity {
                tag,
                generation,
                session,
                signals,
                window,
                ..
            } => LoadResult::ActivityResolved {
                tag,
                generation,
                window,
                result: session.resolve_activity(&signals, window.0, window.1),
            },
        }
    }
}

pub enum LoadResult<Tag = u64> {
    Opened {
        tag: Tag,
        generation: u64,
        result: anyhow::Result<Arc<dyn Session>>,
    },
    ActivityBuilt {
        tag: Tag,
        generation: u64,
        result: anyhow::Result<()>,
    },
    Track {
        tag: Tag,
        generation: u64,
        request_id: u64,
        track: crate::data::transactions::TrackRef,
        result: anyhow::Result<crate::data::loaded_tracks::LoadedTrack>,
    },
    Signals {
        tag: Tag,
        generation: u64,
        results: SignalLoads,
    },
    ActivityResolved {
        tag: Tag,
        generation: u64,
        window: (u64, u64),
        result: anyhow::Result<Vec<SignalRef>>,
    },
}
