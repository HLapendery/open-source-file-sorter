FROM rust:1.88-bookworm AS builder

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --locked --release --bin sorter

FROM debian:bookworm-slim

COPY --from=builder /build/target/release/sorter /usr/local/bin/sorter
RUN useradd --create-home --uid 1000 sorter \
    && mkdir -p /config /data /files \
    && chown -R sorter:sorter /config /data /files

USER sorter
WORKDIR /config
ENTRYPOINT ["sorter"]
CMD ["--help"]
