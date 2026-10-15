# syntax=docker/dockerfile:1
#
# vmangos (World of Warcraft 1.12.1, build 5875) built from source at a pinned commit.
#
# One image plays every vmangos role in compose.yaml: realmd, mangosd, the one-shot database
# initialiser (db-init) and the one-shot map extractor (extract). The role is the command.
#
# The build follows upstream's own release pipeline (.github/workflows/development-release.yaml):
# Ubuntu 24.04, libmysqlclient, OpenSSL, zlib, no ACE or TBB any more. Upstream's
# contrib/docker-build predates that and no longer configures the tree.
#
# It builds natively on linux/amd64 and linux/arm64 (an Apple-silicon Mac under Docker Desktop or
# OrbStack, a Raspberry Pi 4/5, Graviton). The published tag is one multi-platform image built on a
# runner of each architecture (.github/workflows/server-images.yml), so Docker pulls the right half.
#
# Build context is the server directory (compose.yaml does this):
#   docker build -f docker/vmangos/Dockerfile --build-arg VMANGOS_COMMIT=<sha> -t vanillish-vmangos .

ARG UBUNTU_VERSION=24.04
ARG VMANGOS_REPO=https://github.com/vmangos/core.git

# ---- compile ------------------------------------------------------------------------------------
FROM ubuntu:${UBUNTU_VERSION} AS build
ARG DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        cmake \
        git \
        libmysqlclient-dev \
        libssl-dev \
        zlib1g-dev \
    && rm -rf /var/lib/apt/lists/*

ARG VMANGOS_REPO
ARG VMANGOS_COMMIT
# A full commit, fetched on its own: a branch name would make the build depend on the day it ran.
RUN test -n "${VMANGOS_COMMIT}" || { echo "VMANGOS_COMMIT build arg is required" >&2; exit 1; } \
    && git init -q /src \
    && git -C /src remote add origin "${VMANGOS_REPO}" \
    && git -C /src fetch -q --depth 1 origin "${VMANGOS_COMMIT}" \
    && git -C /src checkout -q FETCH_HEAD

# SUPPORTED_CLIENT_BUILD keeps its default (1.12.1). BUILD_FOR_HOST_CPU=0 so the image runs on any
# x86-64 or arm64 (armv8-a: GCC's outline atomics still use LSE where the CPU has it), not just the
# machine that built it. The extractors turn the client's MPQs into the dbc/maps/vmaps/mmaps
# mangosd needs.
#
# Two flags make the arm64 build the same program as the x86-64 one (on x86-64 both change nothing):
#   -fsigned-char      plain `char` is unsigned on Linux aarch64, signed on x86-64 and on Apple's
#                      arm64, where vmangos is developed. Its vendored Recast keeps -1 in a `char`
#                      (rcBuildContours' winding, fixed upstream in recastnavigation 57610fa): without
#                      the flag, MoveMapGenerator never merges contour holes on arm64.
#   -ffp-contract=off  GCC fuses a*b+c into one FMA on aarch64 by default; baseline x86-64 has no
#                      FMA. Off, float arithmetic rounds as on x86-64 (libm may still differ in the
#                      last bit).
# The compile database proves both reached every C and C++ translation unit (745 at 53d24d5c,
# FetchContent'd zstd and libdwarf included), here and on the next core.
#
# BUILD_JOBS caps the parallel compile: a job takes up to ~1.5 GB, and Docker Desktop's VM gets half
# the Mac's memory by default. Empty = one job per core. /build goes once installed, so the layer
# (and the build cache CI keeps of it) holds only the result.
ARG BUILD_JOBS=
ARG VMANGOS_ARCH_FLAGS="-fsigned-char -ffp-contract=off"
RUN cmake -S /src -B /build \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_C_FLAGS="${VMANGOS_ARCH_FLAGS}" \
        -DCMAKE_CXX_FLAGS="${VMANGOS_ARCH_FLAGS}" \
        -DCMAKE_EXPORT_COMPILE_COMMANDS=ON \
        -DCMAKE_INSTALL_PREFIX=/opt/vmangos \
        -DBUILD_EXTRACTORS=1 \
        -DBUILD_FOR_HOST_CPU=0 \
        -DDEBUG_SYMBOLS=0 \
    && { grep -E '"command": .* -c [^ ]+\.(c|cc|cpp|cxx)",?$' /build/compile_commands.json > /tmp/tus \
         || { echo "no C or C++ translation unit in /build/compile_commands.json" >&2; exit 1; }; } \
    && for flag in ${VMANGOS_ARCH_FLAGS}; do \
         ! grep -v -q -e "$flag" /tmp/tus \
           || { echo "a translation unit is compiled without $flag" >&2; exit 1; }; \
       done \
    && rm /tmp/tus \
    && cmake --build /build -j"${BUILD_JOBS:-$(nproc)}" \
    && cmake --install /build \
    && rm -rf /build \
    && mkdir -p /opt/vmangos/sql/migrations \
    && cp /src/sql/migrations/*_world.sql /src/sql/migrations/*_logon.sql \
          /src/sql/migrations/*_characters.sql /src/sql/migrations/*_logs.sql \
          /opt/vmangos/sql/migrations/ \
    && cp /src/LICENSE /opt/vmangos/LICENSE \
    && git -C /src rev-parse HEAD > /opt/vmangos/sql/core-commit

# ---- world database snapshot --------------------------------------------------------------------
# vmangos publishes the world database only as a release asset (tag db_latest), named after the
# commit it was dumped at. Empty VMANGOS_DB_URL means "whatever db_latest holds right now", checked
# against the digest GitHub lists for it; db-init then brings it up to this image's core with the
# core's own migrations. A VMANGOS_DB_URL of your own is checked against VMANGOS_DB_SHA256 when one
# is given. The image records the zip's sha256 either way (CI resolves both once, so the amd64 and
# arm64 halves of a tag carry the same dump). The dump is SQL text, the same on every architecture,
# so this stage runs on the builder's own platform.
FROM --platform=$BUILDPLATFORM ubuntu:${UBUNTU_VERSION} AS db
ARG DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl jq \
    && rm -rf /var/lib/apt/lists/*
ARG VMANGOS_DB_URL=
ARG VMANGOS_DB_SHA256=
RUN url="${VMANGOS_DB_URL}" want="${VMANGOS_DB_SHA256}" \
    && if [ -z "$url" ]; then \
         release="$(curl -fsSL https://api.github.com/repos/vmangos/core/releases/tags/db_latest)" \
         && asset='[.assets[] | select(.name | test("^db-[0-9a-f]+\\.zip$"))][0]' \
         && url="$(printf '%s' "$release" | jq -r "$asset.browser_download_url // empty")" \
         && want="$(printf '%s' "$release" | jq -r "$asset.digest // empty | sub(\"^sha256:\"; \"\")")"; \
       fi \
    && test -n "$url" || { echo "could not find the db_latest release asset" >&2; exit 1; } \
    && echo "world database: $url" \
    && curl -fsSL -o /db.zip "$url" \
    && sha256sum /db.zip | cut -d' ' -f1 > /db.sha256 \
    && echo "world database sha256: $(cat /db.sha256)" \
    && if [ -n "$want" ] && [ "$want" != "$(cat /db.sha256)" ]; then \
         echo "the world database's sha256 is not $want" >&2; exit 1; \
       fi \
    && echo "$url" > /db.url

# ---- runtime ------------------------------------------------------------------------------------
FROM ubuntu:${UBUNTU_VERSION}
ARG DEBIAN_FRONTEND=noninteractive
# mariadb-client and unzip are for db-init, python3 for the mmap generator's driver script.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        libmysqlclient21 \
        libssl3t64 \
        mariadb-client \
        python3 \
        tzdata \
        unzip \
        zlib1g \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build /opt/vmangos /opt/vmangos
COPY --from=db /db.zip /db.url /db.sha256 /opt/vmangos/sql/
COPY docker/vmangos/bin/ /usr/local/bin/

# A checkout with CRLF endings would hand the shell `bash\r`; strip them rather than trust the
# checkout. Then fail the build now, not at first start, if a library the binaries need is missing.
RUN sed -i 's/\r$//' /usr/local/bin/inilla-* /usr/local/bin/vmangos-console \
    && chmod 0755 /usr/local/bin/inilla-* /usr/local/bin/vmangos-console \
    && ! ldd /opt/vmangos/bin/mangosd /opt/vmangos/bin/realmd /opt/vmangos/bin/Extractors/* 2>/dev/null \
         | grep 'not found' \
    && groupadd --system vmangos \
    && useradd --system --gid vmangos --home-dir /vmangos --shell /usr/sbin/nologin vmangos \
    && mkdir -p /vmangos/data /vmangos/logs /run/vmangos \
    && chown -R vmangos:vmangos /vmangos /run/vmangos

# source: where this image is built from; version and revision: the vmangos commit it packages
# (GPLv2, its LICENSE in /opt/vmangos), not Ubuntu's. CI adds the image tag, this repository's
# commit and the dump it baked.
ARG VMANGOS_REPO
ARG VMANGOS_COMMIT
LABEL org.opencontainers.image.title="Vanillish vmangos" \
      org.opencontainers.image.description="vmangos 1.12.1 (build 5875), built from source, for the Vanillish and iNilla clients" \
      org.opencontainers.image.source="https://github.com/klanthier/vanillish" \
      org.opencontainers.image.version="${VMANGOS_COMMIT}" \
      org.opencontainers.image.revision="${VMANGOS_COMMIT}" \
      io.github.vmangos.source="${VMANGOS_REPO}" \
      io.github.vmangos.commit="${VMANGOS_COMMIT}"

ENV PATH=/opt/vmangos/bin:/opt/vmangos/bin/Extractors:${PATH}
USER vmangos
WORKDIR /vmangos
# No default role: compose.yaml names one per service.
CMD ["sh", "-c", "echo 'give a role: inilla-realmd, inilla-mangosd, inilla-db-init or inilla-extract' >&2; exit 2"]
