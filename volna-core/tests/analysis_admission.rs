//! Analysis admission must precede reading histories or allocating derived arrays.

use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};
use volna_core::data::NumericKind;
use volna_core::session::{LoadRequest, LoadResult};
use volna_core::testing::a;
use volna_core::wave::stack::{Layer, Reading};
use volna_trace::data::{Bit, SignalHistory, SignalRef, SignalShape, WaveValue};
use volna_trace::remote::memory::MemoryBudget;

struct ObservedHistory {
    len: usize,
    budget: MemoryBudget,
    reads: AtomicUsize,
    minimum_reserved: AtomicU64,
}

impl ObservedHistory {
    fn observe(&self) {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.minimum_reserved
            .fetch_min(self.budget.used(), Ordering::Relaxed);
    }
}

impl SignalHistory for ObservedHistory {
    fn shape(&self) -> SignalShape {
        SignalShape::Real
    }
    fn len(&self) -> usize {
        self.len
    }
    fn time(&self, i: usize) -> u64 {
        self.observe();
        i as u64
    }
    fn value(&self, _: Option<usize>) -> WaveValue {
        self.observe();
        WaveValue::Real(1.0)
    }
    fn bit(&self, _: Option<usize>) -> Bit {
        self.observe();
        Bit::Other
    }
}

fn request(kind: usize, h: Arc<dyn SignalHistory>, budget: MemoryBudget) -> LoadRequest {
    let reading = Reading::Number(NumericKind::Real);
    match kind {
        0 => LoadRequest::Summary {
            generation: 1,
            signal: a(SignalRef(0)),
            history: h,
            kind: NumericKind::Real,
            budget,
        },
        1 => LoadRequest::GroupSummary {
            generation: 1,
            members: vec![h],
            range: (0, 16_384),
            budget,
        },
        2 => LoadRequest::Integral {
            generation: 1,
            history: h,
            reading,
            budget,
        },
        _ => LoadRequest::StackTotal {
            generation: 1,
            layers: vec![Layer::new(0, h, reading)],
            range: (0, 16_384),
            budget,
        },
    }
}

fn resident_bytes(result: &LoadResult) -> anyhow::Result<u64> {
    match result {
        LoadResult::Summary { result, .. } => result.as_ref().map(|s| s.resident_bytes()),
        LoadResult::GroupSummary { result, .. } => result.as_ref().map(|s| s.resident_bytes()),
        LoadResult::Integral { result, .. } => result.as_ref().map(|s| s.resident_bytes()),
        LoadResult::StackTotal { result, .. } => result.as_ref().map(|s| s.resident_bytes()),
        _ => unreachable!(),
    }
    .map_err(|e| anyhow::anyhow!("{e:#}"))
}

fn admission(kind: usize) {
    for refusal in 0..3 {
        let budget = MemoryBudget::new(if refusal == 0 { 1 } else { 8 << 20 });
        if refusal == 1 {
            budget.set_object_limit(1);
        }
        let held = (refusal == 2).then(|| budget.reserve(budget.limit()).unwrap());
        let h = Arc::new(ObservedHistory {
            len: 16_384,
            budget: budget.clone(),
            reads: AtomicUsize::new(0),
            minimum_reserved: AtomicU64::new(u64::MAX),
        });
        let result = request(kind, h.clone(), budget.clone()).perform();
        assert!(resident_bytes(&result).is_err());
        assert_eq!(
            h.reads.load(Ordering::Relaxed),
            0,
            "refused work must not scan input (case {refusal})"
        );
        drop((result, held));
        assert_eq!(budget.used(), 0);
    }
    for len in [0, 1, 64, 65, 66, 16_384] {
        let budget = MemoryBudget::new(8 << 20);
        let h = Arc::new(ObservedHistory {
            len,
            budget: budget.clone(),
            reads: AtomicUsize::new(0),
            minimum_reserved: AtomicU64::new(u64::MAX),
        });
        let result = request(kind, h.clone(), budget.clone()).perform();
        let bytes = resident_bytes(&result).unwrap();
        assert_eq!(
            budget.used(),
            bytes,
            "retain only resident ownership after construction"
        );
        if h.reads.load(Ordering::Relaxed) > 0 {
            assert!(
                h.minimum_reserved.load(Ordering::Relaxed) >= bytes,
                "construction must hold admission"
            );
        }
        drop(result);
        assert_eq!(
            budget.used(),
            0,
            "dropping a completed result releases its admission"
        );
    }
}

#[test]
fn analog_admission_precedes_analysis() {
    admission(0);
}
#[test]
fn group_admission_precedes_analysis() {
    admission(1);
}
#[test]
fn integral_admission_precedes_analysis() {
    admission(2);
}
#[test]
fn total_admission_precedes_analysis() {
    admission(3);
}

struct TrackingAllocator;
thread_local! {
    static LARGEST_ALLOCATION: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}
fn observe_allocation(bytes: usize) {
    let _ = LARGEST_ALLOCATION.try_with(|peak| {
        if let Some(previous) = peak.get() {
            peak.set(Some(previous.max(bytes)));
        }
    });
}
unsafe impl std::alloc::GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        observe_allocation(layout.size());
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, new_size: usize) -> *mut u8 {
        observe_allocation(new_size);
        unsafe { std::alloc::System.realloc(ptr, layout, new_size) }
    }
}
#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

fn assert_refused_without_arrays(request: LoadRequest) {
    LARGEST_ALLOCATION.with(|peak| peak.set(Some(0)));
    let result = request.perform();
    let peak = LARGEST_ALLOCATION.with(|peak| peak.replace(None).unwrap());
    let refused = match result {
        LoadResult::ActivityCounter { result, .. } => result.is_err(),
        LoadResult::Activity { result, .. } | LoadResult::ActivityResolved { result, .. } => {
            result.is_err()
        }
        _ => unreachable!(),
    };
    assert!(refused);
    assert!(
        peak < 1024,
        "refused analysis allocated a {peak}-byte array"
    );
}

fn activity_session() -> (tempfile::NamedTempFile, Arc<dyn volna_trace::Session>) {
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut w = vtr::Writer::create(file.path()).unwrap();
    let mut signals = Vec::new();
    for i in 0..1024 {
        let scope = w
            .add_scope(None, &format!("scope{i}"), vtr::ScopeType::Module, "")
            .unwrap();
        let (_, signal) = w
            .add_var(
                Some(scope),
                "bit",
                vtr::VarType::Wire,
                vtr::Direction::Input,
                vtr::SignalKind::Bits {
                    width: 1,
                    states: 2,
                },
            )
            .unwrap();
        signals.push(signal);
    }
    for time in [0, 10, 20] {
        w.set_time(time).unwrap();
        for &signal in &signals {
            w.emit_bit(signal, vtr::Logic::from(time == 10)).unwrap();
        }
    }
    w.close().unwrap();
    let reader = vtr::Reader::open(file.path()).unwrap();
    let id = vtr::activity::Identity::of(&reader).unwrap();
    vtr::activity::Sidecar::new(file.path(), &id, None)
        .write(|out| vtr::activity::build(&reader, out, &vtr::activity::BuildOptions::default()))
        .unwrap();
    let session = volna_trace::session::OpenSpec::Path(file.path().into())
        .open()
        .unwrap();
    (file, session)
}

fn counter(session: &Arc<dyn volna_trace::Session>) -> Arc<volna_core::data::ActivityCounter> {
    match (LoadRequest::ActivityCounter {
        trace: volna_core::trace::TraceId::A,
        generation: 1,
        session: session.clone(),
        budget: MemoryBudget::new(8 << 20),
    })
    .perform()
    {
        LoadResult::ActivityCounter { result, .. } => result.unwrap(),
        _ => unreachable!(),
    }
}

#[test]
fn activity_counter_admission_precedes_array_allocation() {
    let mut h = volna_trace::data::source::HierarchyBuilder::default();
    for i in 0..1024 {
        h.push_scope(format!("scope{i}"), "module".into(), None);
    }
    let session = volna_core::testing::hierarchy_session(h);
    assert_refused_without_arrays(LoadRequest::ActivityCounter {
        trace: volna_core::trace::TraceId::A,
        generation: 1,
        session,
        budget: MemoryBudget::new(1),
    });
}

#[test]
fn activity_classification_admission_precedes_array_allocation() {
    let (_file, session) = activity_session();
    assert_refused_without_arrays(LoadRequest::Activity {
        trace: volna_core::trace::TraceId::A,
        generation: 1,
        index: session.activity().unwrap(),
        counter: counter(&session),
        window: (1, 9),
        budget: MemoryBudget::new(1),
    });
}

#[test]
fn activity_resolution_admission_precedes_array_allocation() {
    let (_file, session) = activity_session();
    let counter = counter(&session);
    let counts = match (LoadRequest::Activity {
        trace: volna_core::trace::TraceId::A,
        generation: 1,
        index: session.activity().unwrap(),
        counter: counter.clone(),
        window: (1, 9),
        budget: MemoryBudget::new(8 << 20),
    })
    .perform()
    {
        LoadResult::Activity { result, .. } => result.unwrap(),
        _ => unreachable!(),
    };
    assert_refused_without_arrays(LoadRequest::ResolveActivity {
        trace: volna_core::trace::TraceId::A,
        generation: 1,
        session,
        counter,
        counts,
        budget: MemoryBudget::new(1),
    });
}
