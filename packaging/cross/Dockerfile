# Compilation croisée de RaspyTV pour Raspberry Pi OS 64 bits (Debian 13 « trixie », aarch64).
#   docker build -t raspytv-cross packaging/cross
#   docker run --rm -v "$PWD:/src" -v raspytv-target:/build raspytv-cross
FROM debian:trixie
ENV DEBIAN_FRONTEND=noninteractive
RUN dpkg --add-architecture arm64 \
 && apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl build-essential pkg-config gcc-aarch64-linux-gnu libc6-dev-arm64-cross \
      libwebkit2gtk-4.1-dev:arm64 libgtk-3-dev:arm64 libgtk-layer-shell-dev:arm64 libudev-dev:arm64 \
      libsoup-3.0-dev:arm64 libjavascriptcoregtk-4.1-dev:arm64 \
 && rm -rf /var/lib/apt/lists/*
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --component clippy \
 && rustup target add aarch64-unknown-linux-gnu
ENV PKG_CONFIG_ALLOW_CROSS=1 \
    PKG_CONFIG_PATH=/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/share/pkgconfig \
    PKG_CONFIG_SYSROOT_DIR=/ \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
    CARGO_TARGET_DIR=/build/target
WORKDIR /src
CMD ["sh", "-c", "cargo clippy -p raspytv --target aarch64-unknown-linux-gnu --release -- -D warnings && cargo build -p raspytv --target aarch64-unknown-linux-gnu --release && mkdir -p dist/aarch64 && cp /build/target/aarch64-unknown-linux-gnu/release/raspytv /build/target/aarch64-unknown-linux-gnu/release/raspytv-kiosk dist/aarch64/"]
