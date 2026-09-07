# syntax=docker/dockerfile:1
# stichos render harness — HAR-03, concept D14 (fixed font set) / D15 (pinned
# engines) / INV-4 (fontset_hash is part of the render-cache key).
#
# Base digest recorded from `docker pull ubuntu:24.04` on 2026-09-03 (Fedora 44 host,
# docker 29.6.2): docker.io/library/ubuntu@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517
#
# Build via ./build.sh (tags stichos-harness:<lo_version>); see README.md.

ARG BASE_REF=ubuntu@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517
FROM ${BASE_REF}

# Pinned per D15: LO releases change layout, so the exact build is immutable here.
# LO_SHA256 is the value from the upstream file
# https://download.documentfoundation.org/libreoffice/stable/26.2.5/deb/x86_64/LibreOffice_26.2.5_Linux_x86-64_deb.tar.gz.sha256
ARG LO_VERSION=26.2.5
ARG LO_SHA256=2f03bfb2ac9f33ea7c77331b4b7a23300fb0ed7443566046bf8b5bc51c1bed1e
ARG LO_BASE_URL=https://download.documentfoundation.org/libreoffice/stable
ARG BASE_REF

# Fixed font set (D14). Single source of truth: used by apt and baked into build-info.
# cjk+extra are required by the corpus cjk-text/rtl-text categories (SA-3 finding:
# fonts-noto-core alone ships no CJK; Arabic/Hebrew live in fonts-noto-extra).
ARG FONT_PACKAGES="fonts-crosextra-carlito fonts-crosextra-caladea fonts-liberation fonts-noto-core fonts-noto-cjk fonts-noto-extra"

ENV LANG=C.UTF-8 \
    LC_ALL=C.UTF-8

# poppler (pdftoppm/pdffonts), fontconfig, jq (build-info + CI tooling), no recommends.
# The TDF deb bundle assumes a desktop base and does not pull these X11/NSS/glib
# runtime libs (measured via ldd on oosplash/soffice.bin); they are installed here.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl jq fontconfig poppler-utils \
      libx11-6 libx11-xcb1 libxext6 libxinerama1 libcairo2 libcups2 libdbus-1-3 \
      libfontconfig1 libfreetype6 libglib2.0-0 libnspr4 libnss3 \
      ${FONT_PACKAGES} \
 && fc-cache -f \
 && rm -rf /var/lib/apt/lists/*

# LibreOffice from the official TDF stable deb bundle (not the distro build).
# Verified twice: against the pinned ARG, then against the upstream .sha256 file.
RUN set -eux; \
    tar_name="LibreOffice_${LO_VERSION}_Linux_x86-64_deb.tar.gz"; \
    url="${LO_BASE_URL}/${LO_VERSION}/deb/x86_64/${tar_name}"; \
    curl -fL --retry 3 -o "/tmp/${tar_name}" "$url"; \
    echo "${LO_SHA256}  /tmp/${tar_name}" | sha256sum -c -; \
    curl -fsSL -o /tmp/lo.sha256 "${url}.sha256"; \
    grep -q "^${LO_SHA256}  " /tmp/lo.sha256; \
    ( cd /tmp && sha256sum -c lo.sha256 ); \
    mkdir -p /tmp/lo; \
    tar -xzf "/tmp/${tar_name}" -C /tmp/lo --strip-components=1; \
    apt-get update; \
    dpkg -i /tmp/lo/DEBS/*.deb; \
    apt-get -f install -y; \
    lo_dir="/opt/libreoffice${LO_VERSION%.*}"; \
    ln -sf "${lo_dir}/program/soffice" /usr/local/bin/soffice; \
    ln -sf "${lo_dir}/program/unopkg" /usr/local/bin/unopkg; \
    rm -rf /tmp/lo "/tmp/${tar_name}" /tmp/lo.sha256 /var/lib/apt/lists/*; \
    soffice --version

# Non-root harvester: LO needs a writable HOME for -env:UserInstallation.
RUN useradd --create-home --shell /bin/bash harvester \
 && mkdir -p /work \
 && chown harvester:harvester /work

# Build-info is provenance metadata only. It carries built_at (so the image is NOT
# byte-reproducible) and is deliberately EXCLUDED from the fontset hash, which is
# computed from font file bytes under /usr/share/fonts (+ ~/.fonts) only.
RUN set -eu; \
    mkdir -p /opt/stichos; \
    pop_ver="$(dpkg-query -W -f='${Version}' poppler-utils)"; \
    fonts_json="$(jq -cn '$ARGS.positional' --args ${FONT_PACKAGES})"; \
    cat > /opt/stichos/build-info.json <<EOF
{
  "lo_version": "${LO_VERSION}",
  "poppler_version": "${pop_ver}",
  "image_digest_base": "${BASE_REF}",
  "fonts": ${fonts_json},
  "built_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
EOF

USER harvester
WORKDIR /work
