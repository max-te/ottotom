//! Guards that the conversion allocates nothing per data point.
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

/// Measured on gauges with one string attribute.
#[test]
fn added_points_allocate_nothing() {
    let small = gauge_metrics(16);
    let large = gauge_metrics(256);
    let added_points = 256 - 16;

    let added_allocations = allocations_converting(&large) - allocations_converting(&small);
    assert_eq!(
        added_allocations, 0,
        "{added_allocations} allocations for {added_points} added points"
    );
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
fn added_points_with_typed_attributes_allocate_nothing() {
    let small = gauge_with_typed_attributes(16);
    let large = gauge_with_typed_attributes(256);
    let added_points = 256 - 16;

    let added_allocations = allocations_converting(&large) - allocations_converting(&small);
    assert_eq!(
        added_allocations, 0,
        "{added_allocations} allocations for {added_points} added points"
    );
}
