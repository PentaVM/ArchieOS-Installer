// Batch layer-30 prefix extraction for pinned Prism b10777: one model load, many examples.
// Each example gets a fresh context with exactly the settings of prefix_export.cpp
// (n_ctx 1024, n_batch = n_ubatch = token count, F32 K/V, no flash attention), so the
// captured rows must be bit-identical to the single-example shim (checked by verify-batch).
// Manifest: one example per line, tab-separated: out_dir \t prompt_bytes \t text_bytes (both
// hex-encoded UTF-8). Writes out_dir/prefix.f32 and out_dir/tokens.json, and prints one short
// JSON line per example (stdout stays small for the guarded runner).
#include "llama.h"
#include "ggml-backend.h"
#include <algorithm>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <memory>
#include <sstream>
#include <stdexcept>
#include <string>
#include <vector>

void llama_set_capture_layers(llama_context *, const int32_t *, size_t);
float * llama_get_embeddings_capture_ith(llama_context *, int32_t);

namespace fs = std::filesystem;

static std::string unhex(const std::string & hex) {
    if (hex.size() % 2) throw std::runtime_error("odd hex length");
    std::string out(hex.size() / 2, '\0');
    for (size_t i = 0; i < out.size(); ++i) out[i] = static_cast<char>(std::stoi(hex.substr(2*i, 2), nullptr, 16));
    return out;
}

static std::vector<llama_token> tokenize(const llama_vocab * vocab, const std::string & text, int limit) {
    std::vector<llama_token> tokens(limit + 1);
    int count = llama_tokenize(vocab, text.data(), static_cast<int32_t>(text.size()), tokens.data(), limit + 1, true, true);
    if (count <= 0 || count > limit) return {};
    tokens.resize(count);
    return tokens;
}

int main(int argc, char ** argv) {
    try {
        std::string model_path, manifest_path, runtime;
        int max_tokens = 4096;
        for (int i = 1; i < argc; i += 2) {
            if (i + 1 >= argc) throw std::runtime_error("missing argument");
            const std::string key(argv[i]);
            if (key == "--model") model_path = argv[i+1];
            else if (key == "--manifest") manifest_path = argv[i+1];
            else if (key == "--runtime") runtime = argv[i+1];
            else if (key == "--max-tokens") max_tokens = std::stoi(argv[i+1]);
            else throw std::runtime_error("unknown argument");
        }
        if (model_path.empty() || manifest_path.empty() || runtime.empty() || max_tokens < 1 || max_tokens > 4096)
            throw std::runtime_error("required arguments absent or out of bounds");
        ggml_backend_load_all_from_path(runtime.c_str());
        ggml_backend_dev_t device = nullptr;
        for (size_t i = 0; i < ggml_backend_dev_count(); ++i) {
            auto candidate = ggml_backend_dev_get(i);
            if (ggml_backend_dev_type(candidate) == GGML_BACKEND_DEVICE_TYPE_GPU &&
                std::string(ggml_backend_dev_name(candidate)).find("CUDA") != std::string::npos) { device = candidate; break; }
        }
        if (!device) throw std::runtime_error("CUDA backend unavailable; no CPU fallback");
        ggml_backend_dev_t devices[] = {device, nullptr};
        auto mp = llama_model_default_params(); mp.n_gpu_layers = 99; mp.devices = devices;
        std::unique_ptr<llama_model, decltype(&llama_model_free)> model(llama_model_load_from_file(model_path.c_str(), mp), llama_model_free);
        if (!model || llama_model_n_embd(model.get()) != 4096 || llama_model_n_layer(model.get()) != 32) throw std::runtime_error("unexpected model geometry");
        const auto * vocab = llama_model_get_vocab(model.get());
        std::ifstream manifest(manifest_path);
        std::string line;
        while (std::getline(manifest, line)) {
            std::istringstream fields(line);
            std::string out_dir, prompt_hex, text_hex;
            if (!std::getline(fields, out_dir, '\t') || !std::getline(fields, prompt_hex, '\t') || !std::getline(fields, text_hex))
                throw std::runtime_error("malformed manifest line");
            const std::string prompt = unhex(prompt_hex), text = unhex(text_hex);
            auto tokens = tokenize(vocab, text, max_tokens);
            auto prompt_tokens = tokenize(vocab, prompt, max_tokens);
            std::cout << "{\"out\":\"" << out_dir << "\"";
            bool prefix_ok = !tokens.empty() && !prompt_tokens.empty() && prompt_tokens.size() < tokens.size() &&
                             std::equal(prompt_tokens.begin(), prompt_tokens.end(), tokens.begin());
            if (!prefix_ok) { std::cout << ",\"error\":\"length or prompt boundary\"}" << std::endl; continue; }
            const int count = static_cast<int>(tokens.size());
            auto cp = llama_context_default_params();
            cp.n_ctx = std::max(1024, count); cp.n_batch = count; cp.n_ubatch = count;
            cp.n_seq_max = 1; cp.n_threads = 4; cp.n_threads_batch = 4; cp.n_outputs_max = count;
            cp.type_k = GGML_TYPE_F32; cp.type_v = GGML_TYPE_F32;
            cp.flash_attn_type = LLAMA_FLASH_ATTN_TYPE_DISABLED;
            std::unique_ptr<llama_context, decltype(&llama_free)> ctx(llama_init_from_model(model.get(), cp), llama_free);
            if (!ctx) throw std::runtime_error("cannot create native context");
            int32_t layer = 30; llama_set_capture_layers(ctx.get(), &layer, 1);
            auto batch = llama_batch_init(count, 0, 1);
            batch.n_tokens = count;
            for (int i = 0; i < count; ++i) {
                batch.token[i] = tokens[i]; batch.pos[i] = i; batch.n_seq_id[i] = 1;
                batch.seq_id[i][0] = 0; batch.logits[i] = 1;
            }
            const int status = llama_decode(ctx.get(), batch);
            llama_batch_free(batch);
            if (status != 0) throw std::runtime_error("native decode failed");
            fs::create_directories(out_dir);
            const fs::path file = fs::path(out_dir) / "prefix.f32";
            if (fs::exists(file)) throw std::runtime_error("refusing existing output");
            std::ofstream prefix(file, std::ios::binary);
            prefix.exceptions(std::ios::badbit | std::ios::failbit);
            for (int i = 0; i < count; ++i) {
                const float * hidden = llama_get_embeddings_capture_ith(ctx.get(), i);
                if (!hidden) throw std::runtime_error("missing requested output row");
                prefix.write(reinterpret_cast<const char *>(hidden), 4096 * sizeof(float));
            }
            prefix.close();
            std::ofstream token_file(fs::path(out_dir) / "tokens.json");
            token_file << "[";
            for (int i = 0; i < count; ++i) token_file << (i ? "," : "") << tokens[i];
            token_file << "]";
            token_file.close();
            std::cout << ",\"prompt_tokens\":" << prompt_tokens.size() << ",\"count\":" << count << "}" << std::endl;
        }
        return 0;
    } catch (const std::exception & error) {
        std::cerr << "archie-wanda-prefix-batch: " << error.what() << std::endl;
        return 1;
    }
}
