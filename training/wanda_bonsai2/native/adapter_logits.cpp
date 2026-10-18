// Native reference with an optional LoRA candidate for pinned Prism b10777.
// Separate from prefix_export.cpp so the training extraction binary (whose digest keys
// the prefix caches) never changes. Same request format; adds --lora FILE.
// Outputs prefix.f32 (layer-30 capture), norm.f32 and logits.f32 for every row.
#include "llama.h"
#include "ggml-backend.h"
#include <algorithm>
#include <cstdint>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>

void llama_set_capture_layers(llama_context *, const int32_t *, size_t);
float * llama_get_embeddings_capture_ith(llama_context *, int32_t);

namespace fs = std::filesystem;
struct Capture { std::vector<float> normalized; bool invalid = false; };

static bool observe(ggml_tensor * tensor, bool ask, void * opaque) {
    auto & capture = *static_cast<Capture *>(opaque);
    if (std::string(ggml_get_name(tensor)) != "result_norm") return false;
    if (ask) return true;
    if (tensor->type != GGML_TYPE_F32 || !ggml_is_contiguous(tensor) || tensor->ne[0] != 4096) { capture.invalid = true; return false; }
    const auto count = static_cast<size_t>(ggml_nelements(tensor));
    if (capture.normalized.size() + count > 4096U * 4096U) { capture.invalid = true; return false; }
    const auto offset = capture.normalized.size();
    capture.normalized.resize(offset + count);
    ggml_backend_tensor_get(tensor, capture.normalized.data() + offset, 0, count*sizeof(float));
    return true;
}

template<class T> static T read(std::ifstream & stream) {
    T result{};
    if (!stream.read(reinterpret_cast<char *>(&result), sizeof(result))) throw std::runtime_error("truncated request");
    return result;
}

static std::ofstream output(const fs::path & path) {
    if (fs::exists(path)) throw std::runtime_error("refusing existing output");
    std::ofstream stream(path, std::ios::binary);
    if (!stream) throw std::runtime_error("cannot create output");
    stream.exceptions(std::ios::badbit | std::ios::failbit);
    return stream;
}

int main(int argc, char ** argv) {
    try {
        std::string model_path, request_path, runtime, lora_path;
        fs::path out;
        for (int i=1; i<argc; i+=2) {
            if (i+1 >= argc) throw std::runtime_error("missing argument");
            const std::string key(argv[i]);
            if (key=="--model") model_path=argv[i+1];
            else if (key=="--request") request_path=argv[i+1];
            else if (key=="--out") out=argv[i+1];
            else if (key=="--runtime") runtime=argv[i+1];
            else if (key=="--lora") lora_path=argv[i+1];
            else throw std::runtime_error("unknown argument");
        }
        if (model_path.empty() || request_path.empty() || out.empty() || runtime.empty()) throw std::runtime_error("required arguments absent");
        std::ifstream request(request_path, std::ios::binary);
        char magic[8]{}; request.read(magic,8);
        if (std::memcmp(magic,"ARWPRE01",8) || read<uint32_t>(request)!=1) throw std::runtime_error("invalid request version");
        const auto ubatch=read<uint32_t>(request), max_tokens=read<uint32_t>(request), length=read<uint32_t>(request);
        if (ubatch>512 || max_tokens==0 || max_tokens>4096 || length==0 || length>131072) throw std::runtime_error("request bounds");
        std::string text(length,'\0');
        if (!request.read(text.data(),length) || request.peek()!=std::ifstream::traits_type::eof()) throw std::runtime_error("request payload mismatch");
        fs::create_directories(out);
        ggml_backend_load_all_from_path(runtime.c_str());
        ggml_backend_dev_t device=nullptr;
        for (size_t i=0; i<ggml_backend_dev_count(); ++i) {
            auto candidate=ggml_backend_dev_get(i);
            if (ggml_backend_dev_type(candidate)==GGML_BACKEND_DEVICE_TYPE_GPU &&
                std::string(ggml_backend_dev_name(candidate)).find("CUDA")!=std::string::npos) { device=candidate; break; }
        }
        if (!device) throw std::runtime_error("CUDA backend unavailable; no CPU fallback");
        ggml_backend_dev_t devices[]={device,nullptr};
        auto mp=llama_model_default_params(); mp.n_gpu_layers=99; mp.devices=devices;
        std::unique_ptr<llama_model,decltype(&llama_model_free)> model(llama_model_load_from_file(model_path.c_str(),mp),llama_model_free);
        if (!model || llama_model_n_embd(model.get())!=4096 || llama_model_n_layer(model.get())!=32) throw std::runtime_error("unexpected model geometry");
        const auto * vocab=llama_model_get_vocab(model.get());
        const int vocabulary=llama_vocab_n_tokens(vocab);
        if (vocabulary!=248320) throw std::runtime_error("unexpected vocabulary");
        std::unique_ptr<llama_adapter_lora,decltype(&llama_adapter_lora_free)> adapter(nullptr,llama_adapter_lora_free);
        if (!lora_path.empty()) {
            adapter.reset(llama_adapter_lora_init(model.get(),lora_path.c_str()));
            if (!adapter) throw std::runtime_error("LoRA candidate rejected by native loader");
        }
        std::vector<llama_token> tokens(max_tokens);
        int count=llama_tokenize(vocab,text.data(),static_cast<int32_t>(text.size()),tokens.data(),max_tokens,true,true);
        if (count<=0 || count>static_cast<int>(max_tokens)) throw std::runtime_error("token sequence outside supported limit");
        tokens.resize(count);
        Capture capture{};
        auto cp=llama_context_default_params();
        cp.n_ctx=std::max<uint32_t>(1024,count); cp.n_batch=count; cp.n_ubatch=ubatch ? std::min<uint32_t>(ubatch,count) : count;
        cp.n_seq_max=1; cp.n_threads=4; cp.n_threads_batch=4; cp.n_outputs_max=count;
        cp.type_k=GGML_TYPE_F32; cp.type_v=GGML_TYPE_F32;
        cp.flash_attn_type=LLAMA_FLASH_ATTN_TYPE_DISABLED;
        cp.cb_eval=observe; cp.cb_eval_user_data=&capture;
        std::unique_ptr<llama_context,decltype(&llama_free)> ctx(llama_init_from_model(model.get(),cp),llama_free);
        if (!ctx) throw std::runtime_error("cannot create native context");
        if (adapter) {
            llama_adapter_lora * list[]={adapter.get()};
            float scales[]={1.0f};
            if (llama_set_adapters_lora(ctx.get(),list,1,scales)!=0) throw std::runtime_error("cannot attach LoRA candidate");
        }
        int32_t layer=30; llama_set_capture_layers(ctx.get(),&layer,1);
        auto batch=llama_batch_init(count,0,1);
        batch.n_tokens=count;
        for (int i=0;i<count;++i) {
            batch.token[i]=tokens[i]; batch.pos[i]=i; batch.n_seq_id[i]=1;
            batch.seq_id[i][0]=0; batch.logits[i]=1;
        }
        int status=llama_decode(ctx.get(),batch);
        llama_batch_free(batch);
        if (status!=0 || capture.invalid) throw std::runtime_error("native decode/capture failed");
        if (capture.normalized.size()!=static_cast<size_t>(count)*4096) throw std::runtime_error("incomplete normalized output rows");
        auto prefix=output(out/"prefix.f32"), logits=output(out/"logits.f32");
        for (int i=0;i<count;++i) {
            const float * hidden=llama_get_embeddings_capture_ith(ctx.get(),i);
            const float * scores=llama_get_logits_ith(ctx.get(),i);
            if (!hidden || !scores) throw std::runtime_error("missing requested output row");
            prefix.write(reinterpret_cast<const char *>(hidden),4096*sizeof(float));
            logits.write(reinterpret_cast<const char *>(scores),vocabulary*sizeof(float));
        }
        auto norm=output(out/"norm.f32");
        norm.write(reinterpret_cast<const char *>(capture.normalized.data()),capture.normalized.size()*sizeof(float));
        std::cout << "{\"tokens\":[";
        for (int i=0;i<count;++i) std::cout << (i ? "," : "") << tokens[i];
        std::cout << "],\"rows\":" << count << ",\"hidden_size\":4096,\"vocabulary\":" << vocabulary
                  << ",\"ubatch\":" << cp.n_ubatch << ",\"lora\":" << (adapter ? "true" : "false") << "}" << std::endl;
        return 0;
    } catch (const std::exception & error) {
        std::cerr << "archie-wanda-adapter: " << error.what() << std::endl;
        return 1;
    }
}
