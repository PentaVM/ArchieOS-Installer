//! What any voice does: say a piece of text, handing its audio over as it is made, and
//! stop the moment it is told to.
//!
//! Two voices stand behind this. Magpie (NVIDIA, through NeMo-Speech.cpp) is the one the
//! plan is built on, and needs a GPU to keep up with speech. Piper (C++, ONNX Runtime) runs
//! in real time on a laptop CPU. The conversation does not know which it has.

/// How one piece was said.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Said {
    pub sample_rate: u32,
    /// Seconds of audio produced.
    pub audio_s: f64,
    /// Seconds it took to produce them.
    pub elapsed_s: f64,
    /// Milliseconds until the first audio was ready.
    pub first_audio_ms: f64,
    /// Whether the listener stopped it before the end.
    pub stopped: bool,
}

/// A voice.
pub trait Speaks: Send {
    /// The rate of the mono PCM16 it produces.
    fn sample_rate(&self) -> u32;

    /// Says `text`, handing each piece of audio to `hear` as it is ready. `hear` returns
    /// false to stop: a person started talking.
    fn say(&mut self, text: &str, hear: &mut dyn FnMut(&[i16]) -> bool) -> Result<Said, String>;
}
