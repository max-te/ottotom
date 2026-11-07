//! Guards the number of allocations the conversion makes per data point.
//!
//! Counting allocations needs a global allocator, which needs `unsafe`, which
//! the workspace otherwise denies. The allocator lives in its own test binary
//! so it counts nothing but this test.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use opentelemetry_sdk::metrics::data::ResourceMetrics;
use ottotom::convert::WriteOpenMetrics;
use ottotom_testsupport::resource_metrics::gauge_metrics;

/// Allocations per added data point, measured on a gauge with one string
/// attribute. The one allocation is the label list that `write_attrs_tuple`
/// collects per point.
const ALLOCATIONS_PER_POINT: usize = 1;

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

#[test]
fn allocations_per_point_do_not_grow() {
    let small = gauge_metrics(16);
    let large = gauge_metrics(256);
    let added_points = 256 - 16;

    let added_allocations = allocations_converting(&large) - allocations_converting(&small);
    let per_point = added_allocations.div_ceil(added_points);
    assert!(
        per_point <= ALLOCATIONS_PER_POINT,
        "{added_allocations} allocations for {added_points} added points"
    );
}
