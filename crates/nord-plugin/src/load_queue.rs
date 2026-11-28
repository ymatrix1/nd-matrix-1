//! The bounded array queue between the editor / library worker (producers) and `process()`
//! (consumer) for [`LoadJob`]s, D218: `try_send` / `try_recv` neither lock nor allocate.

use crate::loader::LoadJob;
use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};

/// Producer end, owned by the editor.
#[derive(Clone)]
pub struct LoadSender(Sender<LoadJob>);

/// Consumer end, owned by the plugin and drained at the start of every `process()` block.
#[derive(Clone)]
pub struct LoadReceiver(Receiver<LoadJob>);

/// New queue with `capacity` slots.
pub fn load_queue(capacity: usize) -> (LoadSender, LoadReceiver) {
    let (tx, rx) = bounded(capacity);
    (LoadSender(tx), LoadReceiver(rx))
}

/// Why [`LoadSender::push`] did not queue a job. The job travels back by value, like the
/// queue's own slots hold it.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum Refused {
    /// The queue is full: here is the job back, for once `process()` has drained some.
    Full(LoadJob),
    /// Nobody will ever drain the queue: the plugin instance that owned it is gone.
    Closed,
}

impl LoadSender {
    /// Queues one job; says why when it cannot.
    #[allow(clippy::result_large_err)]
    pub fn push(&self, job: LoadJob) -> Result<(), Refused> {
        match self.0.try_send(job) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(j)) => Err(Refused::Full(j)),
            Err(TrySendError::Disconnected(_)) => Err(Refused::Closed),
        }
    }
}

impl LoadReceiver {
    /// Next pending job. Never blocks, never allocates.
    pub fn pop(&self) -> Option<LoadJob> {
        self.0.try_recv().ok()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nord_core::render_opts::FirmwareOptions;

    #[test]
    fn the_queue_is_bounded_and_hands_a_rejected_job_back() {
        let (tx, rx) = load_queue(2);
        assert!(tx
            .push(LoadJob::Firmware(FirmwareOptions::default()))
            .is_ok());
        assert!(tx
            .push(LoadJob::Piano {
                slot_b: false,
                sound: None
            })
            .is_ok());
        let back = tx.push(LoadJob::Sample {
            layer: 0,
            sound: None,
        });
        assert!(matches!(back, Err(Refused::Full(LoadJob::Sample { .. }))));
        assert!(matches!(rx.pop(), Some(LoadJob::Firmware(_))));
        assert!(matches!(rx.pop(), Some(LoadJob::Piano { .. })));
        assert!(rx.pop().is_none() && rx.is_empty());
        // A queue nobody drains any more says so, instead of handing the job back.
        drop(rx);
        assert!(matches!(
            tx.push(LoadJob::WaveBank(true)),
            Err(Refused::Closed)
        ));
    }
}
