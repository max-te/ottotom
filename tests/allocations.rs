//! Guards that the conversion's allocations amortize: they must not grow with
//! the number of data points.
//!
//! Counting allocations needs a global allocator, which needs `unsafe`, which
//! the workspace otherwise denies. The allocator lives in its own test binary
//! so it counts nothing but this test.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use opentelemetry::{KeyValue, StringValue, Value};
use opentelemetry_sdk::metrics::data::ResourceMetrics;
use ottotom::convert::WriteOpenMetrics;
use ottotom_testsupport::resource_metrics::{collect_metrics, gauge_metrics};

struct CountingAllocator;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

/// Allocations made while converting `metrics` into a buffer that is already large enough.
fn allocations_converting(metrics: &ResourceMetrics) -> usize {
    let mut output = String::with_capacity(1 << 20);
    let before = ALLOCATIONS.load(Ordering::Relaxed);
    metrics.write_as_openmetrics(&mut output).unwrap();
    ALLOCATIONS.load(Ordering::Relaxed) - before
}

/// Allocations that converting more points may add, because buffers grow when
/// an estimate of their size falls short. One allocation per point would add
/// hundreds.
const GROWTH_ALLOWANCE: usize = 8;

/// Asserts that 1024 points cost at most a few allocations more than 64. The
/// SDK keeps at most 2000 attribute sets per instrument.
fn assert_allocations_amortize(metrics: fn(usize) -> ResourceMetrics) {
    let (small, large) = (64, 1024);
    let added_allocations =
        allocations_converting(&metrics(large)) - allocations_converting(&metrics(small));
    assert!(
        added_allocations <= GROWTH_ALLOWANCE,
        "{added_allocations} allocations for {} added points",
        large - small
    );
}

/// Measured on gauges with one string attribute.
#[test]
fn allocations_amortize_with_string_attributes() {
    assert_allocations_amortize(gauge_metrics);
}

/// A gauge whose points carry one attribute of every non-string value type.
fn gauge_with_typed_attributes(points: usize) -> ResourceMetrics {
    collect_metrics(|meter| {
        let gauge = meter.f64_gauge("typed").build();
        for i in 0..points {
            let n = i64::try_from(i).unwrap();
            gauge.record(
                1.0,
                &[
                    KeyValue::new("int", n),
                    KeyValue::new("float", n as f64 / 4.0),
                    KeyValue::new("bool", i % 2 == 0),
                    KeyValue::new("ints", Value::Array(vec![n, -n].into())),
                    KeyValue::new(
                        "strings",
                        Value::Array(
                            vec![StringValue::from("a\"b"), StringValue::from("c")].into(),
                        ),
                    ),
                ],
            );
        }
    })
}

#[test]
fn allocations_amortize_with_typed_attributes() {
    assert_allocations_amortize(gauge_with_typed_attributes);
}
