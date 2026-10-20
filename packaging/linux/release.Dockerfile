FROM ghcr.io/astral-sh/uv:0.9.5 AS uv
FROM ubuntu:22.04

ARG DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl git build-essential cmake pkg-config clang libclang-dev \
    libssl-dev libcurl4-openssl-dev libfontconfig1-dev libfreetype6-dev \
    libx11-xcb-dev libxcb1-dev libxcb-shape0-dev libxcb-xfixes0-dev \
    libxcb-randr0-dev libxcb-render0-dev libxcb-sync-dev libxcb-xkb-dev \
    libxcb-composite0-dev libxcb-damage0-dev libxcb-present-dev \
    libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev \
    libasound2-dev libudev-dev file squashfs-tools \
    && rm -rf /var/lib/apt/lists/*

COPY --from=uv /uv /usr/local/bin/uv
ENV UV_PYTHON_INSTALL_DIR=/opt/python UV_PYTHON_BIN_DIR=/usr/local/bin
RUN uv python install 3.13 --default

# Pass the Mac's exact compiler version; never silently switch stable between
# releases or share incompatible macOS/Linux target directories.
ARG RUST_VERSION
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo
ENV PATH=/opt/cargo/bin:$PATH
RUN test -n "$RUST_VERSION" \
    && curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs -o /tmp/rustup.sh \
    && sh /tmp/rustup.sh -y --profile minimal --default-toolchain "$RUST_VERSION" \
       --component rustfmt --component clippy \
    && rm /tmp/rustup.sh \
    && rustc --version && cargo --version

WORKDIR /src
