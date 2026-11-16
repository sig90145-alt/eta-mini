package main

import (
	"fmt"

	ort "github.com/yalue/onnxruntime_go"
)

// ONNX 모델과의 계약 (training/export_onnx.py 참고): 입력 "X" [N, 2] float32 → 출력 "variable" [N, 1].
// 정규화(StandardScaler)는 모델 안에 있으므로 원래 단위 값을 그대로 넣는다.
const (
	inputName  = "X"
	outputName = "variable"
)

// onnxPredictor 는 ONNX Runtime 세션 하나를 모든 요청이 함께 쓴다.
// DynamicAdvancedSession 은 호출마다 입출력 텐서를 따로 받으므로 동시에 불러도 안전하다.
type onnxPredictor struct {
	session *ort.DynamicAdvancedSession
}

// newONNXPredictor 는 ONNX Runtime 공유 라이브러리를 불러 환경을 만들고 모델을 연다.
// libPath 가 비어 있으면 onnxruntime_go 의 기본 이름으로 찾는다.
func newONNXPredictor(libPath, modelPath string) (*onnxPredictor, error) {
	if libPath != "" {
		ort.SetSharedLibraryPath(libPath)
	}
	if err := ort.InitializeEnvironment(); err != nil {
		return nil, fmt.Errorf("ONNX Runtime 초기화 실패 (lib=%q): %w", libPath, err)
	}
	if err := checkModelSignature(modelPath); err != nil {
		_ = ort.DestroyEnvironment()
		return nil, err
	}
	session, err := ort.NewDynamicAdvancedSession(modelPath, []string{inputName}, []string{outputName}, nil)
	if err != nil {
		_ = ort.DestroyEnvironment()
		return nil, fmt.Errorf("모델 로드 실패 (%s): %w", modelPath, err)
	}
	return &onnxPredictor{session: session}, nil
}

// checkModelSignature 는 모델의 입출력 이름·타입이 계약과 맞는지 시작할 때 확인한다.
// 다른 모델 파일이 들어가면 요청을 받기 전에 바로 실패하게 한다.
func checkModelSignature(modelPath string) error {
	inputs, outputs, err := ort.GetInputOutputInfo(modelPath)
	if err != nil {
		return fmt.Errorf("모델 정보 읽기 실패 (%s): %w", modelPath, err)
	}
	if len(inputs) != 1 || inputs[0].Name != inputName || inputs[0].DataType != ort.TensorElementDataTypeFloat ||
		len(inputs[0].Dimensions) != 2 || inputs[0].Dimensions[1] != 2 {
		return fmt.Errorf("모델 입력이 계약(%q float32 [N, 2])과 다르다: %v", inputName, inputs)
	}
	for _, o := range outputs {
		if o.Name == outputName && o.DataType == ort.TensorElementDataTypeFloat {
			return nil
		}
	}
	return fmt.Errorf("모델 출력에 %q float32 가 없다: %v", outputName, outputs)
}

func (p *onnxPredictor) Predict(distanceKm, pendingOrders float32) (float32, error) {
	input, err := ort.NewTensor(ort.NewShape(1, 2), []float32{distanceKm, pendingOrders})
	if err != nil {
		return 0, fmt.Errorf("입력 텐서 생성 실패: %w", err)
	}
	defer input.Destroy()
	output, err := ort.NewEmptyTensor[float32](ort.NewShape(1, 1))
	if err != nil {
		return 0, fmt.Errorf("출력 텐서 생성 실패: %w", err)
	}
	defer output.Destroy()

	if err := p.session.Run([]ort.Value{input}, []ort.Value{output}); err != nil {
		return 0, fmt.Errorf("추론 실패: %w", err)
	}
	return output.GetData()[0], nil
}

func (p *onnxPredictor) Close() {
	_ = p.session.Destroy()
	_ = ort.DestroyEnvironment()
}
