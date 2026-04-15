FROM rust:bookworm

ARG DEBIAN_FRONTEND=noninteractive
ENV PATH="/usr/local/cargo/bin:${PATH}"

RUN apt-get update && apt-get install -y --no-install-recommends \
    clang \
    curl \
    git \
    pkg-config \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Prefer prebuilt binaries for the heavyweight cargo subcommands so local
# preflight cold-starts do not spend minutes compiling tooling from source.
# cargo-binstall itself is bootstrapped from its published release installer.
RUN curl -L --proto '=https' --tlsv1.2 -sSf \
    https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh \
    | bash
RUN cargo binstall --no-confirm cargo-deny@0.19.0 || cargo install --locked cargo-deny@0.19.0
RUN cargo binstall --no-confirm cargo-audit@0.22.1 || cargo install --locked cargo-audit@0.22.1
RUN cargo binstall --no-confirm cargo-nextest@0.9.132 || cargo install --locked cargo-nextest@0.9.132
RUN cargo binstall --no-confirm cargo-llvm-cov@0.8.5 || cargo install --locked cargo-llvm-cov@0.8.5
# The published GNU binary currently requires a newer glibc than bookworm ships,
# so install cargo-mutants from source in the canonical container.
RUN cargo install --locked cargo-mutants@27.0.0

# Allow git operations on the mounted workspace (ownership mismatch in CI)
RUN git config --global --add safe.directory /workspace/batpak

WORKDIR /workspace/batpak
