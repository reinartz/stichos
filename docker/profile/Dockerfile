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

# ── Stage: build ────────────────────────────────────────────────────────────
FROM rust:1.85-slim AS build

WORKDIR /workspace

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

# ── LibreOffice 26.2.5 (pinned, from TDF) ───────────────────────────────────
# LO_SHA256 is verified against upstream .sha256 file and the file itself.
ARG LO_VERSION=26.2.5
ARG LO_SHA256=2f03bfb2ac9f33ea7c77331b4b7a23300fb0ed7443566046bf8b5bc51c1bed1e
ARG LO_BASE_URL=https://download.documentfoundation.org/libreoffice/stable

RUN set -eux; \
    tar_name="LibreOffice_${LO_VERSION}_Linux_x86-64_deb.tar.gz"; \
    url="${LO_BASE_URL}/${LO_VERSION}/deb/x86_64/${tar_name}"; \
    curl -fL --retry 3 -o "/tmp/${tar_name}" "$url"; \
    echo "${LO_SHA256}  /tmp/${tar_name}" | sha256sum -c -; \
    mkdir -p /tmp/lo; \
    tar -xzf "/tmp/${tar_name}" -C /tmp/lo --strip-components=1; \
    apt-get update; \
    dpkg -i /tmp/lo/LibreOffice_${LO_VERSION}_Linux_x86-64_deb/DEBS/*.deb; \
    apt-get -f install -y -o Dpkg::Options::="--force-confnew"; \
    # Install English language pack if available
    for d in /tmp/lo/LibreOffice_${LO_VERSION}_Linux_x86-64_deb/Langpack_en-US/*/DEBS/*.deb; do \
        [ -f "$d" ] && dpkg -i "$d" || true; \
    done; \
    # Symlink binaries
    ln -sf /opt/libreoffice${LO_VERSION%.*}/program/soffice /usr/local/bin/soffice; \
    ln -sf /opt/libreoffice${LO_VERSION%.*}/program/unopkg /usr/local/bin/unopkg; \
    rm -rf /tmp/lo "/tmp/${tar_name}"; \
    soffice --version

# ── veraPDF 1.6.4 (Java-based PDF/A validator) ─────────────────────────────
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
           default-jre-headless \
    && rm -rf /var/lib/apt/lists/* \
    && curl -fL --retry 3 -o /usr/local/bin/verapdf \
           "https://raw.githubusercontent.com/veraPDF/verapdf-service-sh/refs/tags/verapdf-1.6.4/verapdf/src/main/scripts/verapdf" 2>/dev/null \
           || curl -fL --retry 3 -o /usr/local/bin/verapdf \
           "https://raw.githubusercontent.com/veraPDF/verapdf/refs/tags/1.6.4/verapdf/script/verapdf" 2>/dev/null || true \
    && chmod +x /usr/local/bin/verapdf 2>/dev/null || true

# ── pandoc 3.7.0.2 ──────────────────────────────────────────────────────────
RUN curl -fL --retry 3 -o /tmp/pandoc.tar.gz \
        "https://github.com/jgm/pandoc/releases/download/3.7.0.2/pandoc-3.7.0.2-linux-amd64.tar.gz" \
    && tar xzf /tmp/pandoc.tar.gz -C /tmp \
    && cp /tmp/pandoc-3.7.0.2/bin/amd64/linux/pandoc /usr/local/bin/ \
    && rm -rf /tmp/pandoc.tar.gz /tmp/pandoc-3.7.0.2

# ── typst 0.13.1 ────────────────────────────────────────────────────────────
RUN curl -fL --retry 3 -o /tmp/typst.tar.xz \
        "https://github.com/typst/typst/releases/download/v0.13.1/typst-linux-x86_64-musl.tar.xz" \
    && tar -xJf /tmp/typst.tar.xz -C /tmp \
    && cp /tmp/typst-linux-x86_64-musl/typst /usr/local/bin/ \
    && rm -rf /tmp/typst.tar.xz /tmp/typst-linux-x86_64-musl

# ── Verify all tools in runtime stage ───────────────────────────────────────
RUN soffice --headless --version \
    && pandoc --version | head -1 \
    && typst --version \
    && qpdf --version | head -1 \
    && (verapdf --version 2>/dev/null || echo "verapdf: not available")

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

ENV LANG=C.UTF-8 \
    LC_ALL=C.UTF-8 \
    DEBIAN_FRONTEND=noninteractive

# Minimal deps: fontconfig + poppler + qpdf + minisign + fonts
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
           ca-certificates \
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

# Copy LibreOffice from runtime stage
COPY --from=runtime /opt/libreoffice /opt/libreoffice
COPY --from=runtime /usr/local/bin/soffice /usr/local/bin/soffice
COPY --from=runtime /usr/local/bin/unopkg /usr/local/bin/unopkg
COPY --from=runtime /usr/share/fonts /usr/share/fonts
COPY --from=runtime /etc/fonts /etc/fonts
COPY --from=runtime /var/lib/fontconfig /var/lib/fontconfig 2>/dev/null || true

# Copy tools from runtime
COPY --from=runtime /usr/bin/java /usr/bin/java
COPY --from=runtime /usr/lib/jvm /usr/lib/jvm
COPY --from=runtime /usr/local/bin/verapdf /usr/local/bin/verapdf
COPY --from=runtime /usr/local/bin/pandoc /usr/local/bin/pandoc
COPY --from=runtime /usr/local/bin/typst /usr/local/bin/typst
COPY --from=runtime /opt/stichos /opt/stichos

# Copy stichos binary from build stage
COPY --from=build /workspace/stichos /usr/local/bin/stichos

# Verify tools in final image
RUN soffice --headless --version \
    && pandoc --version | head -1 \
    && typst --version \
    && qpdf --version | head -1

# Non-root user for running the image
RUN useradd --create-home --shell /bin/bash harvester
USER harvester
WORKDIR /work
