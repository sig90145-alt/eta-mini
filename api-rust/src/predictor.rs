//! ONNX 모델(artifacts/model_sklearn.onnx)을 ONNX Runtime 으로 실행하는 예측기.
//!
//! 모델과의 계약 (training/export_onnx.py 참고): 입력 "X" [N, 2] float32 → 출력 "variable" [N, 1] float32.
//! 정규화(StandardScaler)는 모델 안에 있으므로 원래 단위 값을 그대로 넣는다.

use std::path::Path;
use std::sync::Mutex;

use ort::session::Session;
use ort::value::{Tensor, TensorElementType};

const INPUT_NAME: &str = "X";
const OUTPUT_NAME: &str = "variable";

/// 원래 단위의 입력(거리 km, 대기 주문 수)으로 예상 시간(분)을 계산한다.
/// 테스트에서는 ONNX Runtime 없이 가짜 구현을 넣는다.
pub trait Predictor: Send + Sync + 'static {
    fn predict(&self, distance_km: f32, pending_orders: f32) -> Result<f32, String>;
}

pub struct OnnxPredictor {
    // ort 의 Session::run 은 &mut self 를 받는다 (일부 ONNX Runtime 내부가 스레드 안전하지 않아서).
    // 이 모델은 한 번 실행에 수 마이크로초라 요청마다 잠깐 잠그는 것으로 충분하다.
    session: Mutex<Session>,
}

impl OnnxPredictor {
    /// ONNX Runtime 공유 라이브러리를 열고 모델을 불러온다.
    /// lib_path 가 없으면 ort 의 기본 규칙(ORT_DYLIB_PATH, 없으면 시스템 검색 경로의 libonnxruntime.so)으로 찾는다.
    pub fn new(lib_path: Option<&Path>, model_path: &Path) -> Result<Self, String> {
        let builder = match lib_path {
            Some(path) => ort::init_from(path)
                .map_err(|e| format!("ONNX Runtime 라이브러리를 열지 못했다 ({}): {e}", path.display()))?,
            None => ort::init(),
        };
        let env = builder.build().map_err(|e| format!("ONNX Runtime 초기화 실패: {e}"))?;
        let session = Session::builder(&env)
            .and_then(|mut b| b.commit_from_file(model_path))
            .map_err(|e| format!("모델 로드 실패 ({}): {e}", model_path.display()))?;
        check_signature(&session)?;
        Ok(Self { session: Mutex::new(session) })
    }
}

/// 모델의 입출력 이름·타입이 계약과 맞는지 시작할 때 확인한다.
/// 다른 모델 파일이 들어가면 요청을 받기 전에 바로 실패하게 한다.
fn check_signature(session: &Session) -> Result<(), String> {
    let input_ok = match session.inputs() {
        [input] => {
            let dtype = input.dtype();
            input.name() == INPUT_NAME
                && dtype.tensor_type() == Some(TensorElementType::Float32)
                && dtype.tensor_shape().is_some_and(|s| s.len() == 2 && s[1] == 2)
        }
        _ => false,
    };
    if !input_ok {
        return Err(format!("모델 입력이 계약({INPUT_NAME:?} float32 [N, 2])과 다르다: {:?}", session.inputs()));
    }
    let output_ok = session
        .outputs()
        .iter()
        .any(|o| o.name() == OUTPUT_NAME && o.dtype().tensor_type() == Some(TensorElementType::Float32));
    if !output_ok {
        return Err(format!("모델 출력에 {OUTPUT_NAME:?} float32 가 없다: {:?}", session.outputs()));
    }
    Ok(())
}

impl Predictor for OnnxPredictor {
    fn predict(&self, distance_km: f32, pending_orders: f32) -> Result<f32, String> {
        let input = Tensor::from_array(([1usize, 2], vec![distance_km, pending_orders]))
            .map_err(|e| format!("입력 텐서 생성 실패: {e}"))?;
        // 앞선 요청이 패닉했어도 세션 자체는 멀쩡하므로 잠금을 되살려 계속 쓴다.
        let mut session = self.session.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let outputs = session
            .run(ort::inputs![INPUT_NAME => input])
            .map_err(|e| format!("추론 실패: {e}"))?;
        let (_, data) = outputs[OUTPUT_NAME]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("출력 읽기 실패: {e}"))?;
        data.first().copied().ok_or_else(|| "출력이 비어 있다".to_string())
    }
}
