//! Values from other threads, handled on the shell's thread.

use crate::Shell;
use aegle_platform::WakeHandle;
use aegle_ui::{HandlerResult, Result};
use std::{
    cell::RefCell,
    collections::VecDeque,
    rc::Rc,
    sync::{Arc, Mutex},
};

/// The sending half of [`Shell::channel`]: cloneable and usable from any thread.
pub struct Sender<T> {
    queue: Arc<Mutex<VecDeque<T>>>,
    wake: WakeHandle,
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self {
            queue: self.queue.clone(),
            wake: self.wake.clone(),
        }
    }
}

impl<T: Send> Sender<T> {
    /// Queues `value` for the handler and wakes the shell.
    pub fn send(&self, value: T) {
        self.queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(value);
        self.wake.wake();
    }
}

pub(crate) trait Drain {
    fn drain(&self, shell: &Shell) -> Result<()>;
}

struct Receiver<T, F> {
    queue: Arc<Mutex<VecDeque<T>>>,
    handler: RefCell<F>,
}

impl<T, F, R> Drain for Receiver<T, F>
where
    F: FnMut(&Shell, T) -> R,
    R: HandlerResult,
{
    fn drain(&self, shell: &Shell) -> Result<()> {
        loop {
            // Never hold the lock while the handler runs: it may send again.
            let next = self
                .queue
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pop_front();
            let Some(value) = next else { return Ok(()) };
            let Ok(mut handler) = self.handler.try_borrow_mut() else {
                return Ok(());
            };
            handler(shell, value).into_result()?;
        }
    }
}

pub(crate) fn new<T: Send + 'static, R: HandlerResult>(
    wake: WakeHandle,
    handler: impl FnMut(&Shell, T) -> R + 'static,
) -> (Sender<T>, Rc<dyn Drain>) {
    let queue = Arc::new(Mutex::new(VecDeque::new()));
    let receiver = Receiver {
        queue: queue.clone(),
        handler: RefCell::new(handler),
    };
    (Sender { queue, wake }, Rc::new(receiver))
}
