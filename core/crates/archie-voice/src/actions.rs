//! Tool scheduling, not tool authority. Each handler still validates its own policy.
//! Dropping a pending action aborts its future; an already completed physical operation
//! cannot be undone. No action is retried implicitly, including after a timeout.
use archie_speech::SpeechToolIntentCandidate;
use std::future::Future;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

pub trait ActionDispatcher {
    fn start(&mut self, intent: SpeechToolIntentCandidate) -> PendingAction;
}

pub struct PendingAction {
    results: Receiver<String>,
    abort: Option<Box<dyn FnOnce() + Send>>,
    completed: bool,
}

impl PendingAction {
    pub fn poll(&mut self) -> Option<String> {
        if self.completed {
            return None;
        }
        let result = match self.results.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => {
                "No se confirmó la operación: la tarea terminó sin resultado. No se reintentó."
                    .into()
            }
        };
        self.completed = true;
        Some(result)
    }
}

impl Drop for PendingAction {
    fn drop(&mut self) {
        if let Some(abort) = self.abort.take() {
            abort();
        }
    }
}

/// Compatibility for embedders supplying an immediate, synchronous handler.
/// The interactive CLI uses AsyncActions instead; this adapter may block its caller.
pub struct InlineActions<'a>(pub &'a mut dyn FnMut(&SpeechToolIntentCandidate) -> String);
impl ActionDispatcher for InlineActions<'_> {
    fn start(&mut self, intent: SpeechToolIntentCandidate) -> PendingAction {
        let (send, results) = mpsc::channel();
        let _ = send.send((self.0)(&intent));
        PendingAction {
            results,
            abort: None,
            completed: false,
        }
    }
}

pub struct AsyncActions<F> {
    runtime: tokio::runtime::Handle,
    timeout: Duration,
    handler: F,
}

impl<F> AsyncActions<F> {
    pub fn new(runtime: tokio::runtime::Handle, timeout: Duration, handler: F) -> Self {
        Self {
            runtime,
            timeout: timeout.clamp(Duration::from_millis(1), Duration::from_secs(300)),
            handler,
        }
    }
}

impl<F, Task> ActionDispatcher for AsyncActions<F>
where
    F: FnMut(SpeechToolIntentCandidate) -> Task,
    Task: Future<Output = String> + Send + 'static,
{
    fn start(&mut self, intent: SpeechToolIntentCandidate) -> PendingAction {
        let (send, results) = mpsc::channel();
        let task = (self.handler)(intent);
        let timeout = self.timeout;
        let task =
            self.runtime.spawn(async move {
                let outcome = tokio::time::timeout(timeout, task).await.unwrap_or_else(|_| {
                "No se confirmó la operación: se agotó el tiempo de espera. No se reintentó.".into()
            });
                let _ = send.send(outcome);
            });
        PendingAction {
            results,
            abort: Some(Box::new(move || task.abort())),
            completed: false,
        }
    }
}
