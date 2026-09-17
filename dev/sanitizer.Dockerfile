# Add diagnostic instrumentation without changing the ordinary verification image.
FROM pipesql-verification-rust:1.98.1-time
RUN rustup toolchain install nightly-2026-09-06 --profile minimal --component rust-src --no-self-update
