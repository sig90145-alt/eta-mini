// Command api-go 는 api/app.py 와 같은 API 계약(POST /eta, GET /health)을 Go 로 구현한 배달 예상 시간 서비스다.
// 예측은 ONNX 모델(artifacts/model_sklearn.onnx)을 ONNX Runtime 으로 실행한다.
//
// 환경 변수
//
//	PORT             듣는 포트 (기본 8080)
//	MODEL_DIR        model_sklearn.onnx 가 있는 폴더 (기본 artifacts, Python API 와 같다)
//	ONNXRUNTIME_LIB  libonnxruntime.so 경로 (Docker 이미지에서는 미리 설정된다)
package main

import (
	"context"
	"errors"
	"log"
	"net/http"
	"os"
	"os/signal"
	"path/filepath"
	"syscall"
	"time"
)

func main() {
	if err := run(); err != nil {
		log.Fatal(err)
	}
}

func run() error {
	modelPath := filepath.Join(getenv("MODEL_DIR", "artifacts"), "model_sklearn.onnx")
	predictor, err := newONNXPredictor(os.Getenv("ONNXRUNTIME_LIB"), modelPath)
	if err != nil {
		return err
	}
	defer predictor.Close()

	srv := &http.Server{
		Addr:              ":" + getenv("PORT", "8080"),
		Handler:           newHandler(predictor),
		ReadHeaderTimeout: 5 * time.Second,
		ReadTimeout:       10 * time.Second,
		WriteTimeout:      10 * time.Second,
		IdleTimeout:       60 * time.Second,
	}

	// SIGTERM(docker stop)을 받으면 처리 중인 요청을 마치고 끝낸다.
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	errCh := make(chan error, 1)
	go func() {
		log.Printf("listening on %s (model=%s)", srv.Addr, modelPath)
		errCh <- srv.ListenAndServe()
	}()

	select {
	case err := <-errCh:
		if !errors.Is(err, http.ErrServerClosed) {
			return err
		}
		return nil
	case <-ctx.Done():
	}
	log.Print("shutting down")
	shutdownCtx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	return srv.Shutdown(shutdownCtx)
}

func getenv(key, fallback string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return fallback
}
