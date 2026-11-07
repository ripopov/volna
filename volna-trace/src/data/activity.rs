//! Immutable activity indexes and reader-side sidecar access.

use std::future::Future;

use crate::remote::memory::{MemoryBudget, Reservation};

pub const AUTOMATIC_ACTIVITY_MEMORY: u64 = 256 << 20;

#[derive(Debug)]
pub struct ActivityMemoryRequired(pub vtr::activity::MemoryEstimate);

impl std::fmt::Display for ActivityMemoryRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "activity index needs approval for {} MiB of loading memory",
            self.0.peak.div_ceil(1 << 20)
        )
    }
}
impl std::error::Error for ActivityMemoryRequired {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ActivityLoadPolicy {
    #[default]
    Automatic,
    Approved {
        peak_bytes: u64,
    },
}

impl ActivityLoadPolicy {
    #[must_use]
    pub fn limit(self, configured: u64) -> u64 {
        match self {
            Self::Automatic => configured.min(AUTOMATIC_ACTIVITY_MEMORY),
            Self::Approved { peak_bytes } => peak_bytes,
        }
    }

    pub fn check(
        self,
        estimate: vtr::activity::MemoryEstimate,
        configured: u64,
    ) -> Result<(), ActivityMemoryRequired> {
        if estimate.peak > self.limit(configured) {
            return Err(ActivityMemoryRequired(estimate));
        }
        Ok(())
    }
}

struct SidecarImage {
    bytes: Vec<u8>,
    estimate: vtr::activity::MemoryEstimate,
}

fn find_image<T>(
    sidecar: &vtr::activity::Sidecar,
    identity: &vtr::activity::Identity,
    mut accept: impl FnMut(SidecarImage) -> anyhow::Result<Option<T>>,
) -> anyhow::Result<Option<T>> {
    let mut required_memory = None;
    for path in sidecar.paths() {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let Ok(estimate) = vtr::activity::Index::estimate_memory(&bytes, identity) else {
            continue;
        };
        match accept(SidecarImage { bytes, estimate }) {
            Ok(Some(value)) => return Ok(Some(value)),
            Ok(None) => {}
            Err(error) if error.is::<ActivityMemoryRequired>() => {
                required_memory.get_or_insert(error);
            }
            Err(error) => return Err(error),
        }
    }
    match required_memory {
        Some(error) => Err(error),
        None => Ok(None),
    }
}

fn ready<T>(future: impl Future<Output = T>) -> T {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    match future.as_mut().poll(&mut context) {
        std::task::Poll::Ready(result) => result,
        std::task::Poll::Pending => unreachable!("local activity parsing does not yield"),
    }
}

fn checkpoint(
    control: Option<&vtr::activity::BuildControl>,
) -> std::future::Ready<vtr::Result<()>> {
    std::future::ready(
        if control.is_some_and(vtr::activity::BuildControl::is_cancelled) {
            Err(vtr::Error::State("activity request cancelled"))
        } else {
            Ok(())
        },
    )
}

// Admission and cancellation must stop a load rather than invalidate a reusable sidecar.
fn decode_bounded<T>(
    budget: &MemoryBudget,
    control: Option<&vtr::activity::BuildControl>,
    parse: impl FnOnce(&mut dyn FnMut(u64) -> vtr::Result<()>) -> vtr::Result<T>,
) -> anyhow::Result<Option<(T, Reservation)>> {
    let mut reservation = budget.reserve(0)?;
    let mut refused = None;
    let result = parse(&mut |used| {
        if used > reservation.bytes()
            && let Err(error) = reservation.grow(used - reservation.bytes())
        {
            refused = Some(error);
            return Err(vtr::Error::State("activity memory admission refused"));
        }
        Ok(())
    });
    if let Some(error) = refused {
        return Err(error);
    }
    if control.is_some_and(vtr::activity::BuildControl::is_cancelled) {
        anyhow::bail!("activity request cancelled");
    }
    Ok(result.ok().map(|value| (value, reservation)))
}

impl SidecarImage {
    fn load(
        self,
        identity: &vtr::activity::Identity,
        policy: ActivityLoadPolicy,
        budget: &MemoryBudget,
        control: Option<&vtr::activity::BuildControl>,
    ) -> anyhow::Result<Option<ActivityIndex>> {
        policy.check(self.estimate, budget.object_limit())?;
        let Some((index, mut reservation)) = decode_bounded(budget, control, |admit| {
            ready(vtr::activity::Index::decode_with(
                &self.bytes,
                identity,
                admit,
                || checkpoint(control),
            ))
        })?
        else {
            return Ok(None);
        };
        drop(self.bytes);
        reservation.shrink(reservation.bytes() - index.memory_bytes())?;
        Ok(Some(ActivityIndex {
            index,
            _reservation: reservation,
        }))
    }

    fn validated(
        self,
        identity: &vtr::activity::Identity,
        budget: &MemoryBudget,
    ) -> anyhow::Result<Option<Vec<u8>>> {
        let valid = decode_bounded(budget, None, |admit| {
            ready(vtr::activity::Index::validate_with(
                &self.bytes,
                identity,
                admit,
                || checkpoint(None),
            ))
        })?;
        Ok(valid.map(|_| self.bytes))
    }
}

#[derive(Debug)]
pub struct ActivityIndex {
    pub(crate) index: vtr::activity::Index,
    pub(crate) _reservation: Reservation,
}

impl std::ops::Deref for ActivityIndex {
    type Target = vtr::activity::Index;
    fn deref(&self) -> &Self::Target {
        &self.index
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActivityBuildInfo {
    pub bytes: u64,
    pub format: vtr::activity::SourceFormat,
}

pub(crate) struct ActivitySource {
    pub path: std::path::PathBuf,
    pub identity: vtr::activity::Identity,
    loaded: std::sync::OnceLock<std::sync::Arc<ActivityIndex>>,
    available: std::sync::atomic::AtomicBool,
}

impl ActivitySource {
    #[cfg(not(target_family = "wasm"))]
    pub fn new_temporary(path: &std::path::Path, identity: vtr::activity::Identity) -> Self {
        Self {
            path: path.to_owned(),
            identity,
            loaded: std::sync::OnceLock::new(),
            available: std::sync::atomic::AtomicBool::new(false),
        }
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn new(path: &std::path::Path, identity: vtr::activity::Identity) -> Self {
        let sidecar = vtr::activity::Sidecar::new(
            path,
            &identity,
            vtr::activity::default_cache_dir().as_deref(),
        );
        let budget = MemoryBudget::new(AUTOMATIC_ACTIVITY_MEMORY);
        let result = find_image(&sidecar, &identity, |image| {
            image.validated(&identity, &budget)
        });
        let available = !matches!(&result, Ok(None));
        Self {
            path: path.to_owned(),
            identity,
            loaded: std::sync::OnceLock::new(),
            available: std::sync::atomic::AtomicBool::new(available),
        }
    }

    pub fn available(&self) -> bool {
        self.available.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn index(&self) -> Option<std::sync::Arc<ActivityIndex>> {
        self.loaded.get().map(std::sync::Arc::clone)
    }

    pub fn resident_bytes(&self) -> u64 {
        self.loaded.get().map_or(0, |l| l.memory_bytes())
    }

    pub fn info(&self) -> ActivityBuildInfo {
        ActivityBuildInfo {
            bytes: self.identity.length,
            format: self.identity.format,
        }
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn image(&self, cache_dir: Option<&std::path::Path>) -> anyhow::Result<Vec<u8>> {
        let sidecar = vtr::activity::Sidecar::new(&self.path, &self.identity, cache_dir);
        let budget = MemoryBudget::new(2 << 30);
        find_image(&sidecar, &self.identity, |image| {
            image.validated(&self.identity, &budget)
        })?
        .ok_or_else(|| anyhow::anyhow!("no valid activity sidecar"))
    }

    pub fn build(
        &self,
        options: &vtr::activity::BuildOptions,
        budget: &MemoryBudget,
        cache_dir: Option<&std::path::Path>,
        signals: usize,
        policy: ActivityLoadPolicy,
        build: impl FnOnce(&mut dyn std::io::Write) -> vtr::Result<vtr::activity::Summary>,
    ) -> anyhow::Result<()> {
        if self.loaded.get().is_some() {
            return Ok(());
        }
        let control = options
            .control
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("activity build needs a control"))?;
        if control.is_cancelled() {
            anyhow::bail!("activity build cancelled");
        }
        let sidecar = vtr::activity::Sidecar::new(&self.path, &self.identity, cache_dir);
        #[cfg(not(target_family = "wasm"))]
        let _lock = {
            let mut locked = None;
            for path in sidecar.paths() {
                let mut name = path.as_os_str().to_owned();
                name.push(".lock");
                if let Some(parent) = std::path::Path::new(&name).parent() {
                    drop(std::fs::create_dir_all(parent));
                }
                if let Ok(file) = std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(name)
                {
                    loop {
                        match file.try_lock() {
                            Ok(()) => break,
                            Err(std::fs::TryLockError::WouldBlock) => {
                                if control.is_cancelled() {
                                    anyhow::bail!("activity build cancelled");
                                }
                                std::thread::sleep(std::time::Duration::from_millis(10));
                            }
                            Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
                        }
                    }
                    locked = Some(file);
                    break;
                }
            }
            locked.ok_or_else(|| anyhow::anyhow!("cannot lock the activity sidecar"))?
        };
        let load = |image: SidecarImage| image.load(&self.identity, policy, budget, Some(control));
        let index = if let Some(index) = find_image(&sidecar, &self.identity, load)? {
            index
        } else {
            let (path, _) = sidecar.write_with_control(control, |w| {
                let scratch = options
                    .memory
                    .saturating_add((signals as u64).saturating_mul(24));
                let _reservation = budget
                    .reserve(scratch)
                    .map_err(|e| vtr::Error::Invalid(e.to_string()))?;
                build(w)
            })?;
            self.available
                .store(true, std::sync::atomic::Ordering::Relaxed);
            let bytes = std::fs::read(&path)?;
            let estimate = vtr::activity::Index::estimate_memory(&bytes, &self.identity)?;
            load(SidecarImage { bytes, estimate })?
                .ok_or_else(|| anyhow::anyhow!("built activity sidecar is invalid"))?
        };
        self.available
            .store(true, std::sync::atomic::Ordering::Relaxed);
        control.complete()?;
        drop(self.loaded.set(std::sync::Arc::new(index)));
        Ok(())
    }
}
