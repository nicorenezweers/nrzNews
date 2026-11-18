# syntax=docker/dockerfile:1.10
# nrzNews – Multi-Stage-Build für amd64 und arm64: Oberfläche (Dioxus/WebAssembly) und Programm
# (Rust) in einer Datei. Braucht nur Docker – kein Rust auf dem Rechner.
#   docker build -t nrznews .
#   docker run -p 8340:8340 -v nrznews-data:/data nrznews
# Beide Architekturen (Rust wird quer-kompiliert, nicht emuliert):
#   docker buildx build --platform linux/amd64,linux/arm64 -t <registry>/nrz-news:<version> --push .
# Nur das Programm für beide Architekturen (landet in dist/linux_<arch>/nrznews):
#   docker buildx build --platform linux/amd64,linux/arm64 --target binaries --output type=local,dest=dist .

ARG DEBIAN=debian:bookworm-slim@sha256:7ff6ba31527b9e336162fa061746c366c4b2032f4d2cff2078bea43f78efe3f8

ARG RUST=rust:1.98-bookworm@sha256:93ce27a88655056a51dbdd8f5f2d7ddc071c7b0070fb288a37b5a285fc83971e

# ---------------------------------------------------------------- Oberfläche
# WebAssembly ist überall gleich – einmal auf dem Build-Rechner bauen, für alle Architekturen.
FROM --platform=$BUILDPLATFORM ${RUST} AS ui
RUN rustup target add wasm32-unknown-unknown
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    cargo install dioxus-cli --version 0.7.10 --locked --root /opt/dx
WORKDIR /src
# Die Oberfläche benutzt die Typen der JSON-API aus types/ (Pfad-Abhängigkeit ../types).
COPY types types
COPY ui ui
COPY xtask xtask
# `dx bundle` räumt seinen Zwischenordner nicht auf – vorher leeren, sonst landen alte
# WASM-Dateien aus dem Cache mit im Programm.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/ui/target,id=nrznews-ui-target \
    cargo run --quiet --manifest-path xtask/Cargo.toml --target-dir /tmp/xtask -- css \
 && cd ui && rm -rf target/dx/*/release/web/public \
 && PATH=/opt/dx/bin:$PATH dx bundle --web --release --out-dir /tmp/ui \
 && mkdir -p /out && cp -r /tmp/ui/public /out/public && test -f /out/public/index.html

# ---------------------------------------------------------------- Programm
FROM --platform=$BUILDPLATFORM ${RUST} AS build
ARG TARGETARCH
ARG BUILDARCH
# Querübersetzer, wenn Ziel und Build-Rechner verschieden sind (das gebündelte SQLite und
# ring werden mit dem C-Compiler des Ziels gebaut).
RUN set -eu; \
    case "$TARGETARCH" in \
      amd64) t=x86_64-unknown-linux-gnu; p=x86_64-linux-gnu; pkg="gcc-x86-64-linux-gnu libc6-dev-amd64-cross" ;; \
      arm64) t=aarch64-unknown-linux-gnu; p=aarch64-linux-gnu; pkg="gcc-aarch64-linux-gnu libc6-dev-arm64-cross" ;; \
      *) echo "Architektur $TARGETARCH nicht unterstützt" >&2; exit 1 ;; \
    esac; \
    echo "$t" > /rust-target; \
    if [ "$TARGETARCH" != "$BUILDARCH" ]; then \
      apt-get update && apt-get install -y --no-install-recommends $pkg && rm -rf /var/lib/apt/lists/*; \
      rustup target add "$t"; \
      printf '[target.%s]\nlinker = "%s-gcc"\n' "$t" "$p" >> /usr/local/cargo/config.toml; \
      u=$(echo "$t" | tr - _); \
      printf 'CC_%s=%s-gcc\nAR_%s=%s-ar\n' "$u" "$p" "$u" "$p" > /cross-env; \
    else \
      touch /cross-env; \
    fi
WORKDIR /src
COPY Cargo.toml Cargo.lock build.rs Rocket.toml ./
COPY types types
COPY src src
COPY static static
COPY locales locales
# Oberfläche aus der Stufe oben; build.rs bettet sie ins Programm ein (Text und WebAssembly
# gzip) – der Server braucht dafür keine Dateien mehr.
COPY --from=ui /out/public ui/dist/public
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,id=nrznews-target-$TARGETARCH \
    env $(cat /cross-env) cargo build --release --locked --target "$(cat /rust-target)" \
 && cp "target/$(cat /rust-target)/release/nrznews" /nrznews \
 && mkdir /data-empty

# ---------------------------------------------------------------- Programm allein
# Für die Release-Anhänge: dist/linux_amd64/nrznews und dist/linux_arm64/nrznews.
FROM scratch AS binaries
COPY --from=build /nrznews /nrznews

# ---------------------------------------------------------------- Laufzeit
FROM ${DEBIAN} AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates tzdata \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd --system --gid 1000 nrznews \
 && useradd --system --uid 1000 --gid 1000 --home-dir /data --shell /usr/sbin/nologin nrznews
COPY --from=build --chown=1000:1000 /data-empty /data
COPY --from=build /nrznews /usr/local/bin/nrznews
COPY Rocket.toml /app/Rocket.toml

WORKDIR /app
ENV NRZNEWS_CONFIG=/app/Rocket.toml \
    NRZNEWS_DATA_DIR=/data \
    NRZNEWS_ADDRESS=0.0.0.0 \
    NRZNEWS_PORT=8340 \
    NRZNEWS_LOG=info,rocket::server=warn,rocket::launch::_=warn,rocket::shield=warn
USER nrznews
VOLUME /data
EXPOSE 8340
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
  CMD ["nrznews", "healthcheck"]
STOPSIGNAL SIGTERM
CMD ["nrznews"]
