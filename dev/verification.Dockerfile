# Provisioning uses the network; verification containers run without it.
# Pin the multi-platform base and package snapshot, not a machine's local image ID.
FROM rust:1.98.1-bookworm@sha256:93ce27a88655056a51dbdd8f5f2d7ddc071c7b0070fb288a37b5a285fc83971e

RUN rm -f /etc/apt/sources.list /etc/apt/sources.list.d/debian.sources \
    && echo 'deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/20260920T000000Z bookworm main' > /etc/apt/sources.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends time=1.9-0.2 \
    && rm -rf /var/lib/apt/lists/* \
    && rustup component add --toolchain 1.98.1 clippy rustfmt \
    && rustc +1.98.1 --version \
    && cargo +1.98.1 clippy --version \
    && cargo +1.98.1 fmt --version \
    && cc --version \
    && /usr/bin/time --version
