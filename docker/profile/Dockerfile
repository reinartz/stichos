# syntax=docker/dockerfile:1
# stichos OCI profile image — P3-30, DIS-06
#
# Multi-stage build:
#   build     — compile stichos binary from workspace source
#   runtime   — install LibreOffice, tools, fonts; verify all tools
#   profile   — lean final image with stichos binary + runtime deps
#
# Reuses HAR-03 pinning (LO 26.2.5, font set) and the same font packages
# as docker/harness/Dockerfile so fontset_hash is identical.

# ── Shared build args (visible to all stages via stage-local redeclare) ─────
ARG LO_VERSION=26.2.5

# ── Stage: build ────────────────────────────────────────────────────────────
# Bookworm base matches the runtime/profile stages (glibc 2.36); a Trixie
# build stage produces a binary that requires newer glibc and fails at runtime.
FROM rust:1.98-slim-bookworm AS build

WORKDIR /workspace

COPY Cargo.lock ./
COPY rust-toolchain.toml ./
COPY Cargo.toml ./
COPY crates/ ./crates/

RUN cargo build --release --bin stichos \
    && cp target/release/stichos /workspace/stichos

# ── Stage: runtime ──────────────────────────────────────────────────────────
# Full-featured stage: all tools + build tools for verification.
# The final profile stage copies only what's needed.
FROM debian:bookworm-slim AS runtime

ENV LANG=C.UTF-8 \
    LC_ALL=C.UTF-8 \
    DEBIAN_FRONTEND=noninteractive

# System deps + fonts + fontconfig (single layer, no recommends)
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
           ca-certificates \
           curl \
           fontconfig \
           poppler-utils \
           qpdf \
           minisign \
           fonts-crosextra-carlito \
           fonts-crosextra-caladea \
           fonts-liberation \
           fonts-noto-core \
           fonts-noto-cjk \
           fonts-noto-extra \
    && fc-cache -f \
    && rm -rf /var/lib/apt/lists/*

# ── Runtime deps shared by engine binaries (headless LO needs X libs) ───────
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
           libxinerama1 libxcomposite1 libxdamage1 libxext6 libxfixes3 \
           libxi6 libxrandr2 libxrender1 libxt6 libx11-xcb1 \
           libcups2 libdbus-glib-1-2 libcairo2 libgl1 xz-utils \
    && rm -rf /var/lib/apt/lists/*

# ── LibreOffice 26.2.5 (pinned, from the TDF archive) ───────────────────────
# TDF rotates point releases out of .../stable/; the official archive names files
# by micro build (26.2.5.2) and publishes .asc rather than .sha256. LO_SHA256 is
# the pinned trust anchor (captured from upstream .sha256 while 26.2.5 was stable).
ARG LO_VERSION=26.2.5
ARG LO_BUILD=26.2.5.2
ARG LO_SHA256=2f03bfb2ac9f33ea7c77331b4b7a23300fb0ed7443566046bf8b5bc51c1bed1e
ARG LO_BASE_URL=https://downloadarchive.documentfoundation.org/libreoffice/old

RUN set -eux; \
    tar_name="LibreOffice_${LO_BUILD}_Linux_x86-64_deb.tar.gz"; \
    url="${LO_BASE_URL}/${LO_BUILD}/deb/x86_64/${tar_name}"; \
    curl -fL --retry 3 --retry-all-errors -o "/tmp/${tar_name}" "$url"; \
    echo "${LO_SHA256}  /tmp/${tar_name}" | sha256sum -c -; \
    mkdir -p /tmp/lo; \
    # Upstream tarball dir carries the micro version (e.g. LibreOffice_26.2.5.2_Linux_x86-64_deb)
    base="$(tar tzf "/tmp/${tar_name}" | head -1 | cut -d/ -f1)"; \
    tar -xzf "/tmp/${tar_name}" -C /tmp/lo; \
    [ -d "/tmp/lo/$base/DEBS" ] || { find /tmp/lo -maxdepth 2 -type d | head -20; exit 1; }; \
    apt-get update; \
    dpkg -i "/tmp/lo/$base"/DEBS/*.deb; \
    apt-get -f install -y -o Dpkg::Options::="--force-confnew"; \
    # Install English language pack if available
    for d in "/tmp/lo/$base"/Langpack_en-US/*/DEBS/*.deb; do \
        [ -f "$d" ] && dpkg -i "$d" || true; \
    done; \
    # Symlink binaries
    ln -sf /opt/libreoffice${LO_VERSION%.*}/program/soffice /usr/local/bin/soffice; \
    ln -sf /opt/libreoffice${LO_VERSION%.*}/program/unopkg /usr/local/bin/unopkg; \
    rm -rf /tmp/lo "/tmp/${tar_name}"; \
    soffice --version

# ── veraPDF: honest-absent ──────────────────────────────────────────────────
# The pinned 1.6.4 raw-script URLs publish 404 and the official distribution
# needs an installer flow (Java + installer zip), so the profile image ships
# without veraPDF. PDF/A revalidation falls back to qpdf (doctor reports
# honest-absent otherwise).

# ── pandoc 3.7.0.2 ──────────────────────────────────────────────────────────
# The public releases/download/ URL 504s for this asset (observed 2026-11-08);
# the REST asset endpoint mints a fresh redirect and serves the same bytes, so
# it is used here. Asset 259081863 is pandoc-3.7.0.2-linux-amd64.tar.gz, and
# the SHA256 pins the bytes (the tarball's binary is statically linked).
ARG PANDOC_SHA256=8f8f67fdd540b6519326b0ac49d5c55c5d5d15e43920e80a086e02c8aff83268
RUN set -eux; \
    curl -fL --retry 8 --retry-all-errors --retry-delay 15 --retry-max-time 300 \
        -H 'Accept: application/octet-stream' \
        -o /tmp/pandoc.tar.gz \
        "https://api.github.com/repos/jgm/pandoc/releases/assets/259081863"; \
    echo "${PANDOC_SHA256}  /tmp/pandoc.tar.gz" | sha256sum -c -; \
    tar xzf /tmp/pandoc.tar.gz -C /tmp; \
    cp /tmp/pandoc-3.7.0.2/bin/pandoc /usr/local/bin/; \
    rm -rf /tmp/pandoc.tar.gz /tmp/pandoc-3.7.0.2

# ── typst 0.13.1 ────────────────────────────────────────────────────────────
# Same rationale as pandoc; asset 235463347 is
# typst-x86_64-unknown-linux-musl.tar.xz (musl-static). SHA256 pins the bytes.
ARG TYPST_SHA256=7d214bfeffc2e585dc422d1a09d2b144969421281e8c7f5d784b65fc69b5673f
RUN set -eux; \
    curl -fL --retry 8 --retry-all-errors --retry-delay 15 --retry-max-time 300 \
        -H 'Accept: application/octet-stream' \
        -o /tmp/typst.tar.xz \
        "https://api.github.com/repos/typst/typst/releases/assets/235463347"; \
    echo "${TYPST_SHA256}  /tmp/typst.tar.xz" | sha256sum -c -; \
    tar -xJf /tmp/typst.tar.xz -C /tmp; \
    cp /tmp/typst-x86_64-unknown-linux-musl/typst /usr/local/bin/; \
    rm -rf /tmp/typst.tar.xz /tmp/typst-x86_64-unknown-linux-musl

# ── Verify all tools in runtime stage ───────────────────────────────────────
RUN set -eux; \
    soffice --headless --version; \
    pandoc --version; \
    typst --version; \
    qpdf --version; \
    verapdf --version 2>/dev/null || echo "verapdf: not available"

# ── Build info ──────────────────────────────────────────────────────────────
RUN mkdir -p /opt/stichos \
    && cat > /opt/stichos/build-info.json <<EOF
{
  "lo_version": "${LO_VERSION}",
  "poppler_version": "$(dpkg-query -W -f='${Version}' poppler-utils)",
  "pandoc_version": "$(pandoc --version | head -1 | awk '{print $2}')",
  "typst_version": "$(typst --version | awk '{print $2}')",
  "qpdf_version": "$(qpdf --version | head -1 | sed 's/qpdf //')",
  "fontset_hash": "sha256:6f26ae4f9bdea927cffa0984d48adcb3e4b60b3fbe263c6a44440a5628927509"
}
EOF

# ── Stage: profile (lean final image) ───────────────────────────────────────
FROM debian:bookworm-slim AS profile

# Re-declare so the runtime args of the parent stage are visible here
ARG LO_VERSION

ENV LANG=C.UTF-8 \
    LC_ALL=C.UTF-8 \
    DEBIAN_FRONTEND=noninteractive

# Minimal deps: fontconfig + poppler + qpdf + minisign + fonts,
# plus the headless LibreOffice X/dbus runtime shared by engine binaries
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
           ca-certificates \
           fontconfig \
           poppler-utils \
           qpdf \
           minisign \
           libxinerama1 libxcomposite1 libxdamage1 libxext6 libxfixes3 \
           libxi6 libxrandr2 libxrender1 libxt6 libx11-xcb1 \
           libcups2 libdbus-glib-1-2 libcairo2 libgl1 \
           fonts-crosextra-carlito \
           fonts-crosextra-caladea \
           fonts-liberation \
           fonts-noto-core \
           fonts-noto-cjk \
           fonts-noto-extra \
    && fc-cache -f \
    && rm -rf /var/lib/apt/lists/*

# Copy LibreOffice from runtime stage. The install dir carries the major
# version (e.g. /opt/libreoffice26.2); the trailing slashes copy the matched
# directory's CONTENTS into the same-named destination directory.
COPY --from=runtime "/opt/libreoffice${LO_VERSION%.*}/" "/opt/libreoffice${LO_VERSION%.*}/"
COPY --from=runtime /usr/share/fonts /usr/share/fonts
COPY --from=runtime /etc/fonts /etc/fonts

# The LO wrapper executables resolve oosplash via their own symlink path and
# COPY dereferences symlinks, so recreate the links instead of copying them.
RUN ln -sf "/opt/libreoffice${LO_VERSION%.*}/program/soffice" /usr/local/bin/soffice \
    && ln -sf "/opt/libreoffice${LO_VERSION%.*}/program/unopkg" /usr/local/bin/unopkg

# Copy tools from runtime
COPY --from=runtime /usr/local/bin/pandoc /usr/local/bin/pandoc
COPY --from=runtime /usr/local/bin/typst /usr/local/bin/typst
COPY --from=runtime /opt/stichos /opt/stichos

# Copy stichos binary from build stage
COPY --from=build /workspace/stichos /usr/local/bin/stichos

# Verify tools in final image (no pipes: they would mask a missing binary)
RUN set -eux; \
    soffice --headless --version; \
    pandoc --version; \
    typst --version; \
    qpdf --version

# Non-root user for running the image
RUN useradd --create-home --shell /bin/bash harvester
USER harvester
WORKDIR /work
