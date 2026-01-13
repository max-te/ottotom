
format:
    cargo +nightly fmt
test:
    cargo nextest run
bench:
    cargo test --bench converter -- solo
bench-baseline:
    cargo export ./target/benchmarks -- bench converter
bench-compare:
    cargo bench --bench converter -- compare -t 3 target/benchmarks/converter
