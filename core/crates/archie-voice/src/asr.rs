//! Audio boundary of the existing conversation loop. Implementations only transcribe;
//! they do not decide the floor, call tools or retain conversation memory.

pub struct Heard {
    pub text: String,
    pub is_final: bool,
}

pub trait AsrStream {
    fn push(&mut self, samples: &[f32], rate: u32) -> Result<(), String>;
    fn next(&mut self) -> Result<Option<Heard>, String>;
    fn end_utterance(&mut self) -> Result<(), String>;
}

pub trait SpeechRecognizer {
    fn listen(
        &self,
        language: &str,
        phrases: &[&str],
        boost: f32,
    ) -> Result<Box<dyn AsrStream + '_>, String>;
    fn recognize(&self, samples: &[f32], rate: u32) -> Result<String, String>;
}
