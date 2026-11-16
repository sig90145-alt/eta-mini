//! api/app.py 와 같은 API 계약(POST /eta, GET /health)을 Rust 로 구현한 배달 예상 시간 서비스.
//! 예측은 ONNX 모델(artifacts/model_sklearn.onnx)을 ONNX Runtime 으로 실행한다.
//!
//! 환경 변수
//!   PORT       듣는 포트 (기본 8080)
//!   MODEL_DIR  model_sklearn.onnx 가 있는 폴더 (기본 artifacts, Python API 와 같다)
//!   ORT_LIB    libonnxruntime.so 경로 (Docker 이미지에서는 미리 설정된다)

mod eta;
mod json;
mod predictor;

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use predictor::OnnxPredictor;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let port: u16 = env_or("PORT", "8080").parse().map_err(|e| format!("PORT 가 올바른 포트 번호가 아니다: {e}"))?;
    let model_path = PathBuf::from(env_or("MODEL_DIR", "artifacts")).join("model_sklearn.onnx");
    let ort_lib = std::env::var_os("ORT_LIB").filter(|v| !v.is_empty()).map(PathBuf::from);

    let predictor = OnnxPredictor::new(ort_lib.as_deref(), &model_path)?;
    let app = eta::router(Arc::new(predictor));

    let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| format!("{addr} 에서 듣지 못했다: {e}"))?;
    eprintln!("listening on {addr} (model={})", model_path.display());
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| format!("서버 오류: {e}"))?;
    eprintln!("shut down");
    Ok(())
}

/// SIGTERM(docker stop)이나 Ctrl+C 를 받으면 처리 중인 요청을 마치고 끝낸다.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    eprintln!("shutting down");
}

fn env_or(key: &str, fallback: &str) -> String {
    std::env::var(key).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| fallback.to_owned())
}
