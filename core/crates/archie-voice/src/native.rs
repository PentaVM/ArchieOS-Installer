//! Archie's ears and voice: Nemotron 3.5 ASR and Magpie TTS, through NeMo-Speech.cpp.
//!
//! NVIDIA's native runtime exposes a C ABI of opaque handles and size-prefixed structs;
//! this is that ABI and nothing more, wrapped so that a handle cannot outlive what it was
//! made from and a failure always carries the runtime's own message. No model runs in
//! this process's language: the ears and the voice are C++ behind a stable C interface.
//!
//! The structs below mirror `nemo_speech/asr.h` and `nemo_speech/tts.h` field for field.
//! Those returned *by value* from the library must match exactly, and their sizes are
//! checked against the C compiler's in the tests. Those only passed *by pointer* may be a
//! prefix: the ABI is append-only, and the runtime reads no further than `size` says.

use std::ffi::{c_char, c_void, CStr, CString};
use std::marker::PhantomData;
use std::path::Path;
use std::ptr;

pub use crate::voice::Said;
use crate::voice::Speaks;

#[derive(Debug, thiserror::Error)]
pub enum NativeError {
    #[error("{what}: {message}")]
    Runtime { what: &'static str, message: String },
    #[error("{0} contains a NUL byte")]
    Nul(&'static str),
    #[error("{0} is not valid UTF-8")]
    Path(&'static str),
}

mod ffi {
    use super::*;

    pub type AsrStatus = i32;
    pub type TtsStatus = i32;
    pub const OK: i32 = 0;

    #[repr(C)]
    pub struct Recognizer {
        _private: [u8; 0],
    }
    #[repr(C)]
    pub struct Stream {
        _private: [u8; 0],
    }
    #[repr(C)]
    pub struct AsrResult {
        _private: [u8; 0],
    }
    #[repr(C)]
    pub struct Synthesizer {
        _private: [u8; 0],
    }

    #[repr(C)]
    pub struct AsrBackendConfig {
        pub size: usize,
        pub gpu: i32,
    }
    #[repr(C)]
    pub struct AsrModelConfig {
        pub size: usize,
        pub path: *const c_char,
        pub name: *const c_char,
    }
    #[repr(C)]
    pub struct AsrStreamingConfig {
        pub size: usize,
        pub chunk_size: f32,
        pub ctc_left_padding: f32,
        pub ctc_right_padding: f32,
        pub rnnt_right_context: i32,
    }
    #[repr(C)]
    pub struct AsrEndpointingConfig {
        pub size: usize,
        pub enable: bool,
        pub vad_based: bool,
        pub stop_history_eou_ms: i32,
    }
    #[repr(C)]
    pub struct AsrRecognizerConfig {
        pub size: usize,
        pub backend: *const AsrBackendConfig,
        pub model: *const AsrModelConfig,
        pub streaming: *const AsrStreamingConfig,
        pub decoder: *const c_void,
        pub vad: *const c_void,
        pub endpointing: *const AsrEndpointingConfig,
        pub postproc: *const c_void,
        pub diar: *const c_void,
        pub batching: *const c_void,
    }
    #[repr(C)]
    pub struct AsrSpeechContext {
        pub size: usize,
        pub phrases: *const *const c_char,
        pub phrase_count: usize,
        pub boost: f32,
    }
    /// Returned by value: must match the header exactly.
    #[repr(C)]
    pub struct AsrRecognitionOptions {
        pub size: usize,
        pub request_id: *const c_char,
        pub language_code: *const c_char,
        pub interim_results: bool,
        pub enable_word_time_offsets: bool,
        pub enable_automatic_punctuation: bool,
        pub verbatim_transcripts: bool,
        pub profanity_filter: bool,
        pub stop_history_eou_ms: i32,
        pub speech_contexts: *const AsrSpeechContext,
        pub speech_context_count: usize,
        pub max_alternatives: i32,
        pub enable_speaker_diarization: bool,
        pub max_speaker_count: i32,
    }

    #[repr(C)]
    pub struct TtsModelConfig {
        pub size: usize,
        pub magpie_model: *const c_char,
        pub codec_model: *const c_char,
        pub tokenizer_model_dir: *const c_char,
        pub text_normalizer_model_dir: *const c_char,
    }
    /// Returned by value: must match the header exactly.
    #[repr(C)]
    pub struct TtsRuntimeConfig {
        pub size: usize,
        pub speaker: i32,
        pub threads: i32,
        pub codec_threads: i32,
        pub seed: i32,
        pub steps: i32,
        pub top_k: i32,
        pub chunk_frames: i32,
        pub codec_queue_depth: i32,
        pub codec_history_frames: i32,
        pub codec_future_frames: i32,
        pub window_ms: i32,
        pub temperature: f32,
        pub override_temperature: bool,
        pub cfg_scale: f32,
        pub override_cfg_scale: bool,
        pub use_cfg: bool,
        pub use_local_transformer: bool,
        pub use_kv_cache: bool,
        pub use_stateful_codec: bool,
        pub codec_cpu: bool,
        pub flush_partial_chunk: bool,
        pub verbose: bool,
        pub lt_backend: i32,
        pub sampling_backend: i32,
        pub uma_mode: i32,
        pub longform_mode: i32,
        pub lt_fp32: bool,
    }
    #[repr(C)]
    pub struct TtsSynthesizerConfig {
        pub size: usize,
        pub model: *const TtsModelConfig,
        pub runtime: *const TtsRuntimeConfig,
        pub default_language_code: *const c_char,
        pub default_voice_name: *const c_char,
    }
    /// Returned by value: must match the header exactly.
    #[repr(C)]
    pub struct TtsSynthesisOptions {
        pub size: usize,
        pub request_id: *const c_char,
        pub language_code: *const c_char,
        pub speaker: i32,
        pub seed: i32,
        pub steps: i32,
        pub top_k: i32,
        pub temperature: f32,
        pub override_temperature: bool,
        pub cfg_scale: f32,
        pub override_cfg_scale: bool,
        pub voice_name: *const c_char,
        pub output_sample_rate: i32,
    }
    /// Passed by pointer: a prefix of the header's struct, which the ABI allows.
    #[repr(C)]
    pub struct TtsSynthesisStats {
        pub size: usize,
        pub sample_rate: i32,
        pub generated_frames: i32,
        pub chunks: i32,
        pub e2e_chunks: i32,
        pub samples_written: u64,
        pub tokenizer_ms: f64,
        pub encoder_ms: f64,
        pub audio_s: f64,
        pub elapsed_s: f64,
        pub rtf: f64,
        pub rtfx: f64,
        pub ttfa_ms: f64,
    }

    pub type PcmCallback = extern "C" fn(*const u8, usize, *mut c_void) -> bool;

    #[link(name = "nemo_speech_asr_c")]
    extern "C" {
        pub fn nemo_speech_asr_recognition_options_default() -> AsrRecognitionOptions;
        pub fn nemo_speech_asr_create(
            cfg: *const AsrRecognizerConfig,
            out: *mut *mut Recognizer,
        ) -> AsrStatus;
        pub fn nemo_speech_asr_destroy(recognizer: *mut Recognizer);
        pub fn nemo_speech_asr_streaming_recognize(
            recognizer: *mut Recognizer,
            options: *const AsrRecognitionOptions,
            out: *mut *mut Stream,
        ) -> AsrStatus;
        pub fn nemo_speech_asr_recognize_f32(
            recognizer: *mut Recognizer,
            options: *const AsrRecognitionOptions,
            samples: *const f32,
            n_samples: usize,
            sample_rate: i32,
            out: *mut *mut AsrResult,
        ) -> AsrStatus;
        pub fn nemo_speech_asr_stream_push_f32(
            stream: *mut Stream,
            samples: *const f32,
            n_samples: usize,
            sample_rate: i32,
        ) -> AsrStatus;
        pub fn nemo_speech_asr_stream_force_endpoint(stream: *mut Stream) -> AsrStatus;
        pub fn nemo_speech_asr_stream_finish(stream: *mut Stream) -> AsrStatus;
        pub fn nemo_speech_asr_stream_next(
            stream: *mut Stream,
            out: *mut *mut AsrResult,
        ) -> AsrStatus;
        pub fn nemo_speech_asr_stream_close(stream: *mut Stream);
        pub fn nemo_speech_asr_result_is_final(result: *const AsrResult) -> bool;
        pub fn nemo_speech_asr_result_audio_processed(result: *const AsrResult) -> f32;
        pub fn nemo_speech_asr_result_alternative_count(result: *const AsrResult) -> usize;
        pub fn nemo_speech_asr_result_transcript(
            result: *const AsrResult,
            alt: usize,
        ) -> *const c_char;
        pub fn nemo_speech_asr_result_language_count(result: *const AsrResult, alt: usize)
            -> usize;
        pub fn nemo_speech_asr_result_language_code(
            result: *const AsrResult,
            alt: usize,
            i: usize,
        ) -> *const c_char;
        pub fn nemo_speech_asr_result_destroy(result: *mut AsrResult);
        pub fn nemo_speech_asr_last_error() -> *const c_char;
        pub fn nemo_speech_asr_version() -> *const c_char;
    }

    #[link(name = "nemo_speech_tts")]
    extern "C" {
        pub fn nemo_speech_tts_runtime_config_default() -> TtsRuntimeConfig;
        pub fn nemo_speech_tts_synthesis_options_default() -> TtsSynthesisOptions;
        pub fn nemo_speech_tts_create(
            cfg: *const TtsSynthesizerConfig,
            out: *mut *mut Synthesizer,
        ) -> TtsStatus;
        pub fn nemo_speech_tts_destroy(synthesizer: *mut Synthesizer);
        pub fn nemo_speech_tts_sample_rate(synthesizer: *const Synthesizer) -> i32;
        pub fn nemo_speech_tts_speaker_count(synthesizer: *const Synthesizer) -> i32;
        pub fn nemo_speech_tts_speaker_name(
            synthesizer: *const Synthesizer,
            i: usize,
        ) -> *const c_char;
        pub fn nemo_speech_tts_synthesize_text(
            synthesizer: *mut Synthesizer,
            options: *const TtsSynthesisOptions,
            text: *const c_char,
            callback: PcmCallback,
            user_data: *mut c_void,
            stats_out: *mut TtsSynthesisStats,
        ) -> TtsStatus;
        pub fn nemo_speech_tts_last_error() -> *const c_char;
    }
}

/// A borrowed C string, copied out; empty when the library gave none.
fn owned(text: *const c_char) -> String {
    if text.is_null() {
        return String::new();
    }
    // SAFETY: the library returns NUL-terminated strings valid until its next call on
    // this thread; this copies before any such call.
    unsafe { CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned()
}

fn asr_error(what: &'static str) -> NativeError {
    // SAFETY: returns a thread-local string, copied immediately.
    NativeError::Runtime {
        what,
        message: owned(unsafe { ffi::nemo_speech_asr_last_error() }),
    }
}

fn tts_error(what: &'static str) -> NativeError {
    // SAFETY: as above.
    NativeError::Runtime {
        what,
        message: owned(unsafe { ffi::nemo_speech_tts_last_error() }),
    }
}

fn c_string(value: &str, what: &'static str) -> Result<CString, NativeError> {
    CString::new(value).map_err(|_| NativeError::Nul(what))
}

fn c_path(path: &Path, what: &'static str) -> Result<CString, NativeError> {
    c_string(path.to_str().ok_or(NativeError::Path(what))?, what)
}

/// The version of the native runtime, for logs.
pub fn runtime_version() -> String {
    // SAFETY: a static string.
    owned(unsafe { ffi::nemo_speech_asr_version() })
}

/// A loaded recogniser. One may serve several listeners.
pub struct Ears {
    recognizer: *mut ffi::Recognizer,
}

// SAFETY: the runtime documents one recogniser as safe to use from several threads.
unsafe impl Send for Ears {}
unsafe impl Sync for Ears {}

impl Ears {
    /// Loads a Nemotron ASR GGUF for streaming on the CPU. `right_context` is how many
    /// 80 ms encoder frames of lookahead the model waits for. Fewer answers sooner but
    /// costs more per second of audio: on a Ryzen 5 4500U only 13 kept up with speech
    /// (real-time factor 1.41 at 13, 0.95 at 6, 0.74 at 3, 0.47 at 1).
    pub fn load(model: &Path, right_context: Option<i32>) -> Result<Self, NativeError> {
        Self::load_on(model, right_context, -1)
    }

    /// As [`Self::load`], on GPU `gpu` (a CUDA device index), or the CPU when it is -1. On
    /// a GPU the ears keep up with far less lookahead, so they answer sooner.
    pub fn load_on(model: &Path, right_context: Option<i32>, gpu: i32) -> Result<Self, NativeError> {
        let path = c_path(model, "the ASR model path")?;
        let backend = ffi::AsrBackendConfig {
            size: size_of::<ffi::AsrBackendConfig>(),
            gpu,
        };
        let model = ffi::AsrModelConfig {
            size: size_of::<ffi::AsrModelConfig>(),
            path: path.as_ptr(),
            name: ptr::null(),
        };
        // The library's own low-latency preset, as its live example spells it out.
        let streaming = ffi::AsrStreamingConfig {
            size: size_of::<ffi::AsrStreamingConfig>(),
            chunk_size: 0.16,
            ctc_left_padding: 1.92,
            ctc_right_padding: 1.92,
            rnnt_right_context: right_context.unwrap_or(-1),
        };
        let config = ffi::AsrRecognizerConfig {
            size: size_of::<ffi::AsrRecognizerConfig>(),
            backend: &backend,
            model: &model,
            streaming: &streaming,
            decoder: ptr::null(),
            vad: ptr::null(),
            endpointing: ptr::null(),
            postproc: ptr::null(),
            diar: ptr::null(),
            batching: ptr::null(),
        };
        let mut recognizer = ptr::null_mut();
        // SAFETY: every pointer in `config` outlives the call, which copies them.
        let status = unsafe { ffi::nemo_speech_asr_create(&config, &mut recognizer) };
        if status != ffi::OK || recognizer.is_null() {
            return Err(asr_error("loading the ASR model"));
        }
        Ok(Self { recognizer })
    }

    /// Starts a stream of audio, transcribed as `language` (a locale such as `es-US`, or
    /// `auto` to let the model say which it heard). `phrases` are words the decoder should
    /// favour — names above all, which a Spanish model otherwise hears as other words
    /// ("Archie" came back as "O GRC").
    pub fn listen(
        &self,
        language: &str,
        phrases: &[&str],
        boost: f32,
    ) -> Result<Listening<'_>, NativeError> {
        let language = c_string(language, "the language code")?;
        let phrases: Vec<CString> = phrases
            .iter()
            .map(|phrase| c_string(phrase, "a phrase to favour"))
            .collect::<Result<_, _>>()?;
        let pointers: Vec<*const c_char> = phrases.iter().map(|phrase| phrase.as_ptr()).collect();
        let context = ffi::AsrSpeechContext {
            size: size_of::<ffi::AsrSpeechContext>(),
            phrases: pointers.as_ptr(),
            phrase_count: pointers.len(),
            boost,
        };
        // SAFETY: returns a fully initialised struct by value.
        let mut options = unsafe { ffi::nemo_speech_asr_recognition_options_default() };
        options.language_code = language.as_ptr();
        options.interim_results = true;
        options.enable_automatic_punctuation = true;
        if !pointers.is_empty() {
            options.speech_contexts = &context;
            options.speech_context_count = 1;
        }
        let mut stream = ptr::null_mut();
        // SAFETY: the options, the language, the phrases and the pointers to them all
        // outlive the call, which copies them.
        let status = unsafe {
            ffi::nemo_speech_asr_streaming_recognize(self.recognizer, &options, &mut stream)
        };
        if status != ffi::OK || stream.is_null() {
            return Err(asr_error("starting to listen"));
        }
        Ok(Listening {
            stream,
            _ears: PhantomData,
        })
    }

    /// Transcribes one whole utterance at once — for an offline model such as Parakeet
    /// TDT, which hears the utterance end to end and so gets more of it right than a
    /// streaming model deciding as the words arrive. `language` is a locale, or `None`
    /// for a model that finds the language itself.
    pub fn recognize(
        &self,
        samples: &[f32],
        sample_rate: u32,
        language: Option<&str>,
    ) -> Result<Heard, NativeError> {
        let language = language
            .map(|code| c_string(code, "the language code"))
            .transpose()?;
        // SAFETY: returns a fully initialised struct by value.
        let mut options = unsafe { ffi::nemo_speech_asr_recognition_options_default() };
        if let Some(language) = &language {
            options.language_code = language.as_ptr();
        }
        options.enable_automatic_punctuation = true;
        let mut result = ptr::null_mut();
        // SAFETY: the options and the samples outlive the call; `result` receives a handle
        // that `read_result` destroys.
        let status = unsafe {
            ffi::nemo_speech_asr_recognize_f32(
                self.recognizer,
                &options,
                samples.as_ptr(),
                samples.len(),
                sample_rate as i32,
                &mut result,
            )
        };
        if status != ffi::OK || result.is_null() {
            return Err(asr_error("transcribing the utterance"));
        }
        // SAFETY: a live result handle, owned here.
        Ok(unsafe { read_result(result) })
    }
}

/// Copies what a result says and destroys it.
///
/// # Safety
/// `result` must be a live handle from the runtime, not used afterwards.
unsafe fn read_result(result: *mut ffi::AsrResult) -> Heard {
    let alternatives = ffi::nemo_speech_asr_result_alternative_count(result);
    let text = if alternatives > 0 {
        owned(ffi::nemo_speech_asr_result_transcript(result, 0))
    } else {
        String::new()
    };
    let language = (alternatives > 0 && ffi::nemo_speech_asr_result_language_count(result, 0) > 0)
        .then(|| owned(ffi::nemo_speech_asr_result_language_code(result, 0, 0)))
        .filter(|code| !code.is_empty());
    let heard = Heard {
        text,
        is_final: ffi::nemo_speech_asr_result_is_final(result),
        audio_s: ffi::nemo_speech_asr_result_audio_processed(result),
        language,
    };
    ffi::nemo_speech_asr_result_destroy(result);
    heard
}

impl Drop for Ears {
    fn drop(&mut self) {
        // SAFETY: created by nemo_speech_asr_create; every stream borrows `self`, so none
        // is left.
        unsafe { ffi::nemo_speech_asr_destroy(self.recognizer) };
    }
}

/// What the ears made of the audio so far.
#[derive(Debug, Clone, PartialEq)]
pub struct Heard {
    pub text: String,
    /// The model judged the utterance over; later words start a new one.
    pub is_final: bool,
    /// Seconds of audio the result accounts for.
    pub audio_s: f32,
    /// The language the model detected, when it was asked to.
    pub language: Option<String>,
}

/// One stream of audio being transcribed. Driven from one thread.
pub struct Listening<'a> {
    stream: *mut ffi::Stream,
    _ears: PhantomData<&'a Ears>,
}

// SAFETY: a stream may move between threads; it must only be driven from one at a time,
// which `&mut self` on every method guarantees.
unsafe impl Send for Listening<'_> {}

impl Listening<'_> {
    /// Feeds mono audio at `sample_rate` (8 to 96 kHz; resampled inside).
    pub fn push(&mut self, samples: &[f32], sample_rate: u32) -> Result<(), NativeError> {
        // SAFETY: the slice is valid for its length during the call, which copies it.
        let status = unsafe {
            ffi::nemo_speech_asr_stream_push_f32(
                self.stream,
                samples.as_ptr(),
                samples.len(),
                sample_rate as i32,
            )
        };
        if status != ffi::OK {
            return Err(asr_error("hearing audio"));
        }
        Ok(())
    }

    /// Takes the next result, if one is ready.
    pub fn next(&mut self) -> Result<Option<Heard>, NativeError> {
        let mut result = ptr::null_mut();
        // SAFETY: `result` receives a handle this function owns and destroys below.
        let status = unsafe { ffi::nemo_speech_asr_stream_next(self.stream, &mut result) };
        if status != ffi::OK {
            return Err(asr_error("reading what was heard"));
        }
        if result.is_null() {
            return Ok(None);
        }
        // SAFETY: a live handle, destroyed by `read_result` after its strings are copied.
        Ok(Some(unsafe { read_result(result) }))
    }

    /// Declares the utterance over now, as when the person has clearly stopped.
    pub fn end_utterance(&mut self) -> Result<(), NativeError> {
        // SAFETY: a live stream.
        if unsafe { ffi::nemo_speech_asr_stream_force_endpoint(self.stream) } != ffi::OK {
            return Err(asr_error("ending the utterance"));
        }
        Ok(())
    }

    /// No more audio is coming; what remains can be read with [`Self::next`].
    pub fn finish(&mut self) -> Result<(), NativeError> {
        // SAFETY: a live stream.
        if unsafe { ffi::nemo_speech_asr_stream_finish(self.stream) } != ffi::OK {
            return Err(asr_error("finishing the stream"));
        }
        Ok(())
    }
}

impl Drop for Listening<'_> {
    fn drop(&mut self) {
        // SAFETY: created by nemo_speech_asr_streaming_recognize and closed once.
        unsafe { ffi::nemo_speech_asr_stream_close(self.stream) };
    }
}

/// Where the voice's model files are.
#[derive(Debug, Clone)]
pub struct VoiceFiles<'a> {
    pub magpie: &'a Path,
    pub codec: &'a Path,
    pub tokenizer_dir: &'a Path,
}

/// A loaded Magpie voice.
pub struct Voice {
    synthesizer: *mut ffi::Synthesizer,
    language: CString,
    voice: Option<CString>,
}

// SAFETY: the synthesizer is only driven through `&mut self`.
unsafe impl Send for Voice {}

impl Voice {
    pub fn load(
        files: &VoiceFiles<'_>,
        language: &str,
        voice: Option<&str>,
        threads: i32,
    ) -> Result<Self, NativeError> {
        let magpie = c_path(files.magpie, "the Magpie model path")?;
        let codec = c_path(files.codec, "the codec model path")?;
        let tokenizer = c_path(files.tokenizer_dir, "the tokenizer directory")?;
        let language = c_string(language, "the language code")?;
        let voice = voice
            .map(|name| c_string(name, "the voice name"))
            .transpose()?;
        let model = ffi::TtsModelConfig {
            size: size_of::<ffi::TtsModelConfig>(),
            magpie_model: magpie.as_ptr(),
            codec_model: codec.as_ptr(),
            tokenizer_model_dir: tokenizer.as_ptr(),
            text_normalizer_model_dir: ptr::null(),
        };
        // SAFETY: returns a fully initialised struct by value.
        let mut runtime = unsafe { ffi::nemo_speech_tts_runtime_config_default() };
        if threads > 0 {
            runtime.threads = threads;
            runtime.codec_threads = threads;
        }
        let config = ffi::TtsSynthesizerConfig {
            size: size_of::<ffi::TtsSynthesizerConfig>(),
            model: &model,
            runtime: &runtime,
            default_language_code: language.as_ptr(),
            default_voice_name: voice.as_ref().map_or(ptr::null(), |name| name.as_ptr()),
        };
        let mut synthesizer = ptr::null_mut();
        // SAFETY: every pointer in `config` outlives the call, which copies them.
        let status = unsafe { ffi::nemo_speech_tts_create(&config, &mut synthesizer) };
        if status != ffi::OK || synthesizer.is_null() {
            return Err(tts_error("loading the voice"));
        }
        Ok(Self {
            synthesizer,
            language,
            voice,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        // SAFETY: a live synthesizer.
        unsafe { ffi::nemo_speech_tts_sample_rate(self.synthesizer) }.max(0) as u32
    }

    /// The language the next sentences are said in (a locale such as `es-US` or `en-US`):
    /// the model is multilingual, and a person may switch languages between turns.
    pub fn set_language(&mut self, language: &str) -> Result<(), NativeError> {
        self.language = c_string(language, "the language code")?;
        Ok(())
    }

    /// The voices this model has, by name.
    pub fn voices(&self) -> Vec<String> {
        // SAFETY: a live synthesizer; names are copied before the next call.
        let count = unsafe { ffi::nemo_speech_tts_speaker_count(self.synthesizer) }.max(0) as usize;
        (0..count)
            .map(|i| owned(unsafe { ffi::nemo_speech_tts_speaker_name(self.synthesizer, i) }))
            .collect()
    }

    /// Says `text`, handing each piece of audio (mono PCM16) to `hear` as it is made.
    /// `hear` returns false to stop — a person started talking.
    pub fn say<F: FnMut(&[i16]) -> bool>(
        &mut self,
        text: &str,
        hear: F,
    ) -> Result<Said, NativeError> {
        extern "C" fn trampoline<F: FnMut(&[i16]) -> bool>(
            pcm: *const u8,
            n_bytes: usize,
            user: *mut c_void,
        ) -> bool {
            // SAFETY: `user` is the `&mut (F, bool)` passed below, alive for the whole
            // synthesis; `pcm` holds `n_bytes` bytes of little-endian i16 for this call.
            let (hear, stopped) = unsafe { &mut *(user as *mut (F, bool)) };
            let samples: Vec<i16> = unsafe { std::slice::from_raw_parts(pcm, n_bytes) }
                .chunks_exact(2)
                .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            let go_on = hear(&samples);
            if !go_on {
                *stopped = true;
            }
            go_on
        }

        let text = c_string(text, "the text to say")?;
        // SAFETY: returns a fully initialised struct by value.
        let mut options = unsafe { ffi::nemo_speech_tts_synthesis_options_default() };
        options.language_code = self.language.as_ptr();
        if let Some(voice) = &self.voice {
            options.voice_name = voice.as_ptr();
        }
        let mut stats = ffi::TtsSynthesisStats {
            size: size_of::<ffi::TtsSynthesisStats>(),
            sample_rate: 0,
            generated_frames: 0,
            chunks: 0,
            e2e_chunks: 0,
            samples_written: 0,
            tokenizer_ms: 0.0,
            encoder_ms: 0.0,
            audio_s: 0.0,
            elapsed_s: 0.0,
            rtf: 0.0,
            rtfx: 0.0,
            ttfa_ms: 0.0,
        };
        let mut state: (F, bool) = (hear, false);
        // SAFETY: `state`, `options`, `text` and `stats` outlive the call; the callback
        // only runs during it.
        let status = unsafe {
            ffi::nemo_speech_tts_synthesize_text(
                self.synthesizer,
                &options,
                text.as_ptr(),
                trampoline::<F>,
                &mut state as *mut (F, bool) as *mut c_void,
                &mut stats,
            )
        };
        let stopped = state.1;
        // A request the listener stopped reports cancellation; that is not a failure.
        if status != ffi::OK && !stopped {
            return Err(tts_error("speaking"));
        }
        Ok(Said {
            sample_rate: if stats.sample_rate > 0 {
                stats.sample_rate as u32
            } else {
                self.sample_rate()
            },
            audio_s: stats.audio_s,
            elapsed_s: stats.elapsed_s,
            first_audio_ms: stats.ttfa_ms,
            stopped,
        })
    }
}

impl Speaks for Voice {
    fn sample_rate(&self) -> u32 {
        Voice::sample_rate(self)
    }

    fn say(&mut self, text: &str, hear: &mut dyn FnMut(&[i16]) -> bool) -> Result<Said, String> {
        Voice::say(self, text, |pcm| hear(pcm)).map_err(|error| error.to_string())
    }
}

impl Drop for Voice {
    fn drop(&mut self) {
        // SAFETY: created by nemo_speech_tts_create and destroyed once.
        unsafe { ffi::nemo_speech_tts_destroy(self.synthesizer) };
    }
}

#[cfg(test)]
mod tests {
    use super::ffi;

    /// Sizes the C compiler gives these structs on x86-64 Linux, read from the headers of
    /// the NeMo-Speech.cpp build this binds, printed by compiling `sizeof` of each against
    /// `include/nemo_speech/*.h`. A struct returned by value that differs by one byte
    /// corrupts the stack.
    #[test]
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn structs_returned_by_value_are_the_size_the_library_writes() {
        assert_eq!(
            size_of::<ffi::AsrRecognitionOptions>(),
            ABI.asr_recognition_options
        );
        assert_eq!(size_of::<ffi::TtsRuntimeConfig>(), ABI.tts_runtime_config);
        assert_eq!(
            size_of::<ffi::TtsSynthesisOptions>(),
            ABI.tts_synthesis_options
        );
        assert_eq!(
            size_of::<ffi::AsrRecognizerConfig>(),
            ABI.asr_recognizer_config
        );
        assert_eq!(size_of::<ffi::TtsModelConfig>(), ABI.tts_model_config);
        assert_eq!(
            size_of::<ffi::TtsSynthesizerConfig>(),
            ABI.tts_synthesizer_config
        );
        assert_eq!(
            size_of::<ffi::AsrStreamingConfig>(),
            ABI.asr_streaming_config
        );
    }

    struct Abi {
        asr_recognition_options: usize,
        tts_runtime_config: usize,
        tts_synthesis_options: usize,
        asr_recognizer_config: usize,
        tts_model_config: usize,
        tts_synthesizer_config: usize,
        asr_streaming_config: usize,
    }

    const ABI: Abi = Abi {
        asr_recognition_options: 72,
        tts_runtime_config: 96,
        tts_synthesis_options: 72,
        asr_recognizer_config: 80,
        tts_model_config: 40,
        tts_synthesizer_config: 40,
        asr_streaming_config: 24,
    };
}
