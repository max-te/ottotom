//! Properties of `OpenMetricsExporter`, from its documentation: `text()` is
//! empty before the first export, then returns the last exported rendering;
//! a handle from `text()` is untouched by later exports; clones share one buffer.

use std::collections::HashSet;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Waker};

use ephyra::{Arbitrary, Gen, gens};
use opentelemetry_sdk::metrics::exporter::PushMetricExporter;
use ottotom::convert::{Config, WriteOpenMetrics};
use ottotom::exporter::OpenMetricsExporter;
use ottotom_ephyra_tests::scenario::{Scenario, scenario};
use ottotom_ephyra_tests::sdk::{self, Order};

/// Polls the export future once, because `export` completes without waiting.
fn export(
    exporter: &OpenMetricsExporter,
    metrics: &opentelemetry_sdk::metrics::data::ResourceMetrics,
) {
    let mut future = pin!(exporter.export(metrics));
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(result) => result.expect("exporting into a String cannot fail"),
        Poll::Pending => panic!("export did not complete on its first poll"),
    }
}

/// Collects `scenario` once and returns the metrics with their expected rendering.
fn rendered(
    scenario: &Scenario,
    config: Config,
) -> (opentelemetry_sdk::metrics::data::ResourceMetrics, String) {
    let metrics = sdk::collect(scenario, Order::AsWritten);
    let text = metrics.to_openmetrics_string_with_config(config).unwrap();
    (metrics, text)
}

#[derive(Clone, Debug, Arbitrary)]
enum Op {
    Text(usize),
    Clone(usize),
    Export(usize, #[ephyra(gen = scenario())] Scenario),
}

#[derive(Clone, Debug, Arbitrary)]
struct Session {
    scope_info: bool,
    target_info: bool,
    #[ephyra(gen = gens::vec(ephyra::any::<Op>()).len(0..=12))]
    ops: Vec<Op>,
}

#[ephyra::generator]
fn sessions() -> impl Gen<Value = Session> {
    ephyra::any::<Session>()
}

#[ephyra::test(gen = sessions(), tests = 500)]
fn text_follows_the_last_export(session: Session) {
    let config = Config::builder()
        .scope_info_enabled(session.scope_info)
        .target_info_enabled(session.target_info)
        .build();
    let mut exporters = vec![OpenMetricsExporter::new(config)];
    let mut last_export = String::new();
    // Handles from `text()`, with the text each held when it was taken.
    let mut handles: Vec<(Arc<String>, String)> = Vec::new();

    for (step, op) in session.ops.iter().enumerate() {
        match op {
            Op::Text(i) => {
                let text = exporters[i % exporters.len()].text();
                assert_eq!(
                    *text, last_export,
                    "step {step}: text() is not the last export"
                );
                handles.push((text, last_export.clone()));
            }
            Op::Clone(i) => {
                let clone = exporters[i % exporters.len()].clone();
                exporters.push(clone);
            }
            Op::Export(i, scenario) => {
                let (metrics, text) = rendered(scenario, config);
                export(&exporters[i % exporters.len()], &metrics);
                last_export = text;
            }
        }
        for (n, exporter) in exporters.iter().enumerate() {
            assert_eq!(
                *exporter.text(),
                last_export,
                "step {step}: clone {n} sees another buffer"
            );
        }
        for (handle, taken) in &handles {
            assert_eq!(**handle, *taken, "step {step}: a held handle changed");
        }
    }
}

#[derive(Clone, Debug, Arbitrary)]
struct Race {
    #[ephyra(gen = gens::vec(scenario()).len(1..=6))]
    scenarios: Vec<Scenario>,
    #[ephyra(gen = gens::int(1usize..=3))]
    writers: usize,
    #[ephyra(gen = gens::int(1usize..=3))]
    readers: usize,
}

#[ephyra::generator]
fn races() -> impl Gen<Value = Race> {
    ephyra::any::<Race>()
}

#[ephyra::test(gen = races(), tests = 300)]
fn concurrent_readers_see_only_complete_exports(race: Race) {
    let config = Config::default();
    let exporter = OpenMetricsExporter::new(config);
    let renderings: Vec<_> = race.scenarios.iter().map(|s| rendered(s, config)).collect();
    // Writer `w` exports renderings w, w + writers, w + 2 * writers, ... in order.
    let batches: Vec<Vec<usize>> = (0..race.writers)
        .map(|w| (w..renderings.len()).step_by(race.writers).collect())
        .collect();
    let writing = AtomicBool::new(true);

    let observed: Vec<HashSet<Arc<String>>> = std::thread::scope(|threads| {
        let readers: Vec<_> = (0..race.readers)
            .map(|_| {
                let exporter = exporter.clone();
                let writing = &writing;
                threads.spawn(move || {
                    let mut seen = HashSet::new();
                    loop {
                        let done = !writing.load(Ordering::Acquire);
                        seen.insert(exporter.text());
                        if done {
                            return seen;
                        }
                    }
                })
            })
            .collect();
        let writers: Vec<_> = batches
            .iter()
            .map(|batch| {
                let exporter = exporter.clone();
                let renderings = &renderings;
                threads.spawn(move || {
                    for &i in batch {
                        export(&exporter, &renderings[i].0);
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        writing.store(false, Ordering::Release);
        readers.into_iter().map(|r| r.join().unwrap()).collect()
    });

    let complete: HashSet<&str> = renderings.iter().map(|(_, text)| text.as_str()).collect();
    for text in observed.iter().flatten() {
        assert!(
            text.is_empty() || complete.contains(text.as_str()),
            "a reader saw text that is no complete export:\n{text}"
        );
    }
    let last_of_each_writer: HashSet<&str> = batches
        .iter()
        .filter_map(|batch| batch.last())
        .map(|&i| renderings[i].1.as_str())
        .collect();
    let last = exporter.text();
    let interleaved = observed
        .iter()
        .flatten()
        .any(|text| !text.is_empty() && *text != last);
    ephyra::classify(interleaved, "a reader saw an intermediate export");
    assert!(
        last_of_each_writer.contains(last.as_str()),
        "the final text is not the last export of any writer:\n{last}"
    );
}
