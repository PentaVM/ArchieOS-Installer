// A real person detector, backed by ONNX Runtime.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §7.1
//
// This runs a detection model — a YOLO export, or any single-shot detector with a standard
// output — on a frame and returns boxes for the people it found. It implements
// `PersonDetector`, so the worker's orchestration, tested against a scripted detector, drives
// this without change.
//
// Compiled only when ONNX Runtime is present (`-DSENTRY_WITH_ONNX=ON`), so the ISO build and
// this development host are unaffected. Validated on a machine with a GPU and a model file,
// which is the only place it can be.
//
// # It returns where, never who
//
// The model answers one question: where are the people? A box and a person-confidence, and
// nothing about identity. A detector that returned an identity would be a face recogniser in
// a detector's clothing, and naming is the consent-gated step in `vision/track.hpp` that
// happens far downstream of this. This file has no access to a face database and could not
// name anyone if it tried — which is the point.
//
// Only the "person" class is kept. A COCO model detects eighty things; a Sentry camera cares
// about one, and dropping the other seventy-nine here keeps a car or a dog from ever becoming
// a track.

#include "sentry/runtime/detector.hpp"

#if defined(SENTRY_WITH_ONNX)

#include <onnxruntime_cxx_api.h>

#include <algorithm>
#include <array>
#include <cstdint>
#include <memory>
#include <string>
#include <vector>

namespace sentry::runtime {

namespace {

// The COCO class index for "person". A model trained on COCO puts people at class 0; a model
// with a different label map needs this changed, which is why it is named rather than a bare
// literal in the loop below.
constexpr int kPersonClass = 0;

}  // namespace

class OnnxPersonDetector final : public PersonDetector {
  public:
    // `model_path` is a detection model exported to ONNX. `confidence_floor` is the score
    // below which a detection is dropped before it can become a track.
    OnnxPersonDetector(const std::string& model_path, float confidence_floor)
        : confidence_floor_(confidence_floor),
          env_(ORT_LOGGING_LEVEL_WARNING, "archie-sentry-detector"),
          session_(env_, model_path.c_str(), session_options()) {}

    DetectionSet detect(const Frame& frame) override {
        DetectionSet result;
        if (!frame.has_pixels() || frame.width <= 0 || frame.height <= 0) {
            return result;
        }

        // Preprocess: the model wants a fixed square input, normalised, planar. The letterbox
        // preserves aspect ratio so a wide frame's people are not squashed into false
        // negatives.
        const std::vector<float> input = letterbox_to_chw(frame);

        const std::array<std::int64_t, 4> shape{1, 3, kInputSize, kInputSize};
        Ort::MemoryInfo memory =
            Ort::MemoryInfo::CreateCpu(OrtArenaAllocator, OrtMemTypeDefault);
        Ort::Value tensor = Ort::Value::CreateTensor<float>(
            memory, const_cast<float*>(input.data()), input.size(), shape.data(), shape.size());

        const char* input_names[] = {input_name_.c_str()};
        const char* output_names[] = {output_name_.c_str()};
        auto outputs = session_.Run(Ort::RunOptions{nullptr}, input_names, &tensor, 1,
                                    output_names, 1);

        // Parse the output into person boxes. The exact layout is model-specific; this
        // handles the common [1, N, 85] YOLO shape (cx, cy, w, h, obj, 80 class scores).
        const float* data = outputs.front().GetTensorData<float>();
        const auto info = outputs.front().GetTensorTypeAndShapeInfo();
        const auto dims = info.GetShape();
        if (dims.size() == 3) {
            const std::int64_t rows = dims[1];
            const std::int64_t stride = dims[2];
            for (std::int64_t i = 0; i < rows; ++i) {
                const float* row = data + i * stride;
                const float objectness = row[4];
                const float person_score = row[5 + kPersonClass];
                const float confidence = objectness * person_score;
                if (confidence < confidence_floor_) {
                    continue;
                }
                Detection d;
                d.camera_id = frame.camera_id;
                d.at = frame.captured_at;
                // Model coordinates are in the letterboxed square; normalise straight to
                // 0..1 frame fraction, which is what the tracker associates on.
                d.cx = std::clamp(row[0] / static_cast<float>(kInputSize), 0.0F, 1.0F);
                d.cy = std::clamp(row[1] / static_cast<float>(kInputSize), 0.0F, 1.0F);
                d.person_confidence = confidence;
                result.detections.push_back(d);
            }
        }
        return result;
    }

    float minimum_confidence() const noexcept override { return confidence_floor_; }
    std::string_view model_name() const noexcept override { return "onnx"; }

  private:
    static constexpr int kInputSize = 640;

    static Ort::SessionOptions session_options() {
        Ort::SessionOptions options;
        options.SetIntraOpNumThreads(2);
        options.SetGraphOptimizationLevel(GraphOptimizationLevel::ORT_ENABLE_ALL);
        return options;
    }

    std::vector<float> letterbox_to_chw(const Frame& frame) const {
        // Scale the frame into a kInputSize square keeping aspect ratio, pad the rest grey,
        // and lay it out channel-major and normalised to 0..1. Nearest-neighbour is enough
        // for a detector and avoids a dependency on an image library.
        std::vector<float> out(3 * kInputSize * kInputSize, 0.5F);
        const int channels = frame.format == PixelFormat::Gray8 ? 1 : 3;
        const float scale = std::min(static_cast<float>(kInputSize) / frame.width,
                                     static_cast<float>(kInputSize) / frame.height);
        const int scaled_w = static_cast<int>(frame.width * scale);
        const int scaled_h = static_cast<int>(frame.height * scale);
        const int pad_x = (kInputSize - scaled_w) / 2;
        const int pad_y = (kInputSize - scaled_h) / 2;

        for (int y = 0; y < scaled_h; ++y) {
            for (int x = 0; x < scaled_w; ++x) {
                const int src_x = static_cast<int>(x / scale);
                const int src_y = static_cast<int>(y / scale);
                const std::size_t src = (static_cast<std::size_t>(src_y) * frame.width + src_x) *
                                        channels;
                if (src + channels > frame.pixels.size()) {
                    continue;
                }
                const int dx = x + pad_x;
                const int dy = y + pad_y;
                for (int c = 0; c < 3; ++c) {
                    const int sc = channels == 1 ? 0 : c;
                    // BGR sources are read in reverse channel order so the model always sees
                    // RGB.
                    const int source_channel =
                        frame.format == PixelFormat::Bgr8 ? (2 - sc) : sc;
                    const float value = frame.pixels[src + source_channel] / 255.0F;
                    out[static_cast<std::size_t>(c) * kInputSize * kInputSize +
                        static_cast<std::size_t>(dy) * kInputSize + dx] = value;
                }
            }
        }
        return out;
    }

    float confidence_floor_;
    Ort::Env env_;
    Ort::Session session_;
    std::string input_name_ = "images";
    std::string output_name_ = "output0";
};

std::shared_ptr<PersonDetector> make_onnx_detector(const std::string& model_path,
                                                   float confidence_floor) {
    return std::make_shared<OnnxPersonDetector>(model_path, confidence_floor);
}

}  // namespace sentry::runtime

#endif  // SENTRY_WITH_ONNX
