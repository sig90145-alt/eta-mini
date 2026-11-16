# Rust 추론 서비스 (api/app.py 와 같은 API 계약). 저장소 루트를 빌드 컨텍스트로 쓴다.
#   docker build -f api-rust/Dockerfile -t eta-api-rust .
#   docker run --rm -p 8082:8080 eta-api-rust
# artifacts/model_sklearn.onnx 가 있어야 한다 (uv run --group train python -m training.export_onnx).

ARG RUST_VERSION=1.100
ARG DEBIAN_RELEASE=trixie

# ── 1) ONNX Runtime 공유 라이브러리 ─────────────────────────────
# 버전은 uv.lock 의 onnxruntime 을 그대로 쓴다. Python API 와 같은 런타임으로 실행해야 예측이 같다.
FROM debian:${DEBIAN_RELEASE}-slim AS onnxruntime
ARG TARGETARCH
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
COPY uv.lock /tmp/uv.lock
RUN set -eu; \
    ORT_VERSION="$(awk '{ sub(/\r$/, "") } $0 == "name = \"onnxruntime\"" { found = 1; next } found && $1 == "version" { gsub(/"/, "", $3); print $3; exit }' /tmp/uv.lock)"; \
    [ -n "$ORT_VERSION" ] || { echo "uv.lock 에서 onnxruntime 버전을 찾지 못했다" >&2; exit 1; }; \
    case "$TARGETARCH" in \
      amd64) ORT_ARCH=x64 ;; \
      arm64) ORT_ARCH=aarch64 ;; \
      *) echo "지원하지 않는 TARGETARCH: '${TARGETARCH}' (amd64, arm64 만 지원)" >&2; exit 1 ;; \
    esac; \
    echo "ONNX Runtime ${ORT_VERSION} (linux-${ORT_ARCH})"; \
    curl -fsSL "https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/onnxruntime-linux-${ORT_ARCH}-${ORT_VERSION}.tgz" \
      | tar -xz -C /opt; \
    mkdir -p /opt/onnxruntime; \
    mv "/opt/onnxruntime-linux-${ORT_ARCH}-${ORT_VERSION}/lib" /opt/onnxruntime/lib; \
    rm -rf "/opt/onnxruntime-linux-${ORT_ARCH}-${ORT_VERSION}" /opt/onnxruntime/lib/cmake /opt/onnxruntime/lib/pkgconfig

# ── 2) Rust 빌드 ──────────────────────────────────────────────
# ort 는 load-dynamic 기능으로 libonnxruntime.so 를 실행 중에 연다. 빌드에는 라이브러리가 필요 없고,
# 기본 기능(download-binaries)을 껐으므로 빌드 중에 런타임을 내려받지도 않는다.
FROM rust:${RUST_VERSION}-slim-${DEBIAN_RELEASE} AS build
WORKDIR /src
COPY api-rust/Cargo.toml api-rust/Cargo.lock ./
COPY api-rust/src ./src
# 레지스트리와 target 을 캐시해 소스만 바뀌면 의존성을 다시 컴파일하지 않는다.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked \
    && install -D target/release/eta-api-rust /out/eta-api-rust

# ── 3) 실행 이미지 ─────────────────────────────────────────────
# 빌드 이미지와 같은 Debian 이라 glibc 가 맞는다.
FROM debian:${DEBIAN_RELEASE}-slim
# 폴더째 복사해야 libonnxruntime.so → .so.1 → .so.<버전> 심볼릭 링크가 유지된다 (와일드카드는 링크를 실체로 3번 복사한다)
COPY --from=onnxruntime /opt/onnxruntime/lib/ /opt/onnxruntime/lib/
COPY --from=build /out/eta-api-rust /usr/local/bin/eta-api-rust
COPY artifacts/model_sklearn.onnx /app/artifacts/model_sklearn.onnx
WORKDIR /app
ENV ORT_LIB=/opt/onnxruntime/lib/libonnxruntime.so \
    MODEL_DIR=/app/artifacts \
    PORT=8080
USER nobody
EXPOSE 8080
CMD ["eta-api-rust"]
