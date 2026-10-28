FROM rust:1.90-bookworm

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        libayatana-appindicator3-dev \
        librsvg2-dev \
        libssl-dev \
        libwebkit2gtk-4.1-dev \
    && rm -rf /var/lib/apt/lists/* \
    && rustup component add clippy rustfmt

WORKDIR /work
