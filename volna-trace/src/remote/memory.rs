//! Non-blocking admission for decoded storage shared by UI and background workers.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

const MIB: u64 = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct MemoryBudget(Arc<Budget>);

#[derive(Debug)]
struct Budget {
    limit: AtomicU64,
    object_limit: AtomicU64,
    used: AtomicU64,
}

#[derive(Debug)]
pub struct Reservation {
    budget: MemoryBudget,
    bytes: u64,
}

impl MemoryBudget {
    #[must_use]
    pub fn new(limit: u64) -> Self {
        Self(Arc::new(Budget {
            limit: AtomicU64::new(limit),
            object_limit: AtomicU64::new(u64::MAX),
            used: AtomicU64::new(0),
        }))
    }

    #[must_use]
    pub fn limit(&self) -> u64 {
        self.0.limit.load(Ordering::Relaxed)
    }

    pub fn set_limit(&self, limit: u64) {
        self.0.limit.store(limit, Ordering::Relaxed);
    }

    #[must_use]
    pub fn object_limit(&self) -> u64 {
        self.0.object_limit.load(Ordering::Relaxed)
    }

    pub fn set_object_limit(&self, limit: u64) {
        self.0.object_limit.store(limit, Ordering::Relaxed);
    }

    #[must_use]
    pub fn used(&self) -> u64 {
        self.0.used.load(Ordering::Relaxed)
    }

    pub fn reserve_object(&self, what: &str, bytes: u64) -> anyhow::Result<Reservation> {
        check_object(what, bytes, self.object_limit(), "retry")?;
        self.reserve(bytes)
    }

    pub fn reserve(&self, bytes: u64) -> anyhow::Result<Reservation> {
        self.admit(bytes)?;
        Ok(Reservation {
            budget: self.clone(),
            bytes,
        })
    }

    fn admit(&self, bytes: u64) -> anyhow::Result<()> {
        let limit = self.limit();
        let admitted = self
            .0
            .used
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(bytes).filter(|&next| next <= limit)
            });
        anyhow::ensure!(
            admitted.is_ok(),
            "memory budget exceeded: {} MiB more would pass the {} MiB budget; \
             release retained objects or raise the memory budget",
            bytes.div_ceil(MIB),
            limit / MIB
        );
        Ok(())
    }

    fn release(&self, bytes: u64) {
        self.0
            .used
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_sub(bytes)
            })
            .expect("balanced memory reservation");
    }
}

pub(crate) fn check_object(what: &str, bytes: u64, limit: u64, then: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        bytes <= limit,
        "{what} is {} MiB, over the {} MiB object size limit; \
         raise the object limit and {then}",
        bytes.div_ceil(MIB),
        limit / MIB
    );
    Ok(())
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.release(self.bytes);
    }
}

impl Reservation {
    #[must_use]
    pub fn budget(&self) -> MemoryBudget {
        self.budget.clone()
    }

    pub fn shrink(&mut self, bytes: u64) -> anyhow::Result<()> {
        self.bytes = self
            .bytes
            .checked_sub(bytes)
            .ok_or_else(|| anyhow::anyhow!("reservation shrink exceeds ownership"))?;
        self.budget.release(bytes);
        Ok(())
    }

    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub(crate) fn split(&mut self, bytes: u64) -> anyhow::Result<Self> {
        self.bytes = self
            .bytes
            .checked_sub(bytes)
            .ok_or_else(|| anyhow::anyhow!("reservation split exceeds ownership"))?;
        Ok(Self {
            budget: self.budget.clone(),
            bytes,
        })
    }
    pub(crate) fn grow(&mut self, bytes: u64) -> anyhow::Result<()> {
        let total = self
            .bytes
            .checked_add(bytes)
            .ok_or_else(|| anyhow::anyhow!("memory size overflow"))?;
        self.budget.admit(bytes)?;
        self.bytes = total;
        Ok(())
    }

    pub fn grow_object(&mut self, what: &str, bytes: u64) -> anyhow::Result<()> {
        let total = self
            .bytes
            .checked_add(bytes)
            .ok_or_else(|| anyhow::anyhow!("memory size overflow"))?;
        check_object(what, total, self.budget.object_limit(), "retry")?;
        self.grow(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_admission_never_exceeds_budget_and_releases_every_reservation() {
        let budget = MemoryBudget::new(30);
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let budget = &budget;
                let barrier = &barrier;
                scope.spawn(move || {
                    for _ in 0..100 {
                        let reservation = budget.reserve(10).ok();
                        barrier.wait();
                        assert_eq!(budget.used(), 30);
                        barrier.wait();
                        drop(reservation);
                        barrier.wait();
                        assert_eq!(budget.used(), 0);
                        barrier.wait();
                    }
                });
            }
        });
    }

    #[test]
    fn reservations_are_shared_checked_and_released() {
        let budget = MemoryBudget::new(u64::MAX);
        let other = budget.clone();
        let reservation = budget.reserve(u64::MAX).unwrap();
        assert!(other.reserve(1).is_err());
        assert_eq!(budget.used(), u64::MAX);
        drop(reservation);
        assert_eq!(other.used(), 0);
        let reservation = other.reserve(7).unwrap();
        drop(other);
        assert_eq!(budget.used(), 7);
        drop(reservation);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn a_live_limit_change_admits_or_refuses_only_new_reservations() {
        let budget = MemoryBudget::new(10);
        let held = budget.reserve(8).unwrap();
        let error = budget.reserve(4).unwrap_err().to_string();
        assert!(error.contains("memory budget exceeded"), "{error}");
        assert!(error.contains("raise the memory budget"), "{error}");
        budget.set_limit(12);
        let more = budget.reserve(4).unwrap();
        budget.set_limit(1);
        assert_eq!(budget.used(), 12, "lowering never evicts");
        assert!(budget.reserve(1).is_err());
        drop((held, more));
        budget.reserve(1).unwrap();
    }

    #[test]
    fn split_ownership_releases_independently_without_readmission() {
        let budget = MemoryBudget::new(80);
        let mut parent = budget.reserve(80).unwrap();
        let child = parent.split(30).unwrap();
        assert_eq!(budget.used(), 80);
        assert!(parent.split(51).is_err());
        assert_eq!(parent.bytes(), 50);
        drop(child);
        assert_eq!(budget.used(), 50);
        drop(parent);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn object_growth_checks_live_limits_before_changing_admission() {
        let budget = MemoryBudget::new(100);
        budget.set_object_limit(20);
        let mut held = budget.reserve_object("view", 10).unwrap();
        assert!(held.grow_object("view", 11).is_err());
        assert_eq!(budget.used(), 10);
        held.grow_object("view", 10).unwrap();
        assert_eq!(budget.used(), 20);
        budget.set_object_limit(100);
        assert!(held.grow_object("view", 81).is_err());
        assert_eq!(budget.used(), 20);
        drop(held);
        assert_eq!(budget.used(), 0);
    }
}
