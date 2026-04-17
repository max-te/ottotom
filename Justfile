
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

# Release: bump version, update changelog, commit, tag, publish.
# Usage: just release 0.32.0
release version:
    cargo set-version {{ version }}
    git cliff --tag v{{ version }} --output CHANGELOG.md
    git add CHANGELOG.md Cargo.toml Cargo.lock
    git commit -m "chore(release): v{{ version }}"
    git tag v{{ version }}
    cargo publish
