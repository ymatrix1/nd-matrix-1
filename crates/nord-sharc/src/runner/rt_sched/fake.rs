//! Recording backend for the unit tests: logs every call, can fail one named call.
use super::{PinMode, PriorityClass, SchedBackend, SchedError, ThreadPriority};
use std::cell::RefCell;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    Class(PriorityClass),
    Priority(ThreadPriority),
    Mmcss(String),
    Pin(usize, PinMode),
}

#[derive(Default)]
pub struct FakeBackend {
    calls: RefCell<Vec<Call>>,
    fail: Option<(&'static str, u32)>,
    pub logical: usize,
}

impl FakeBackend {
    /// Fail the call named `call` (`SetThreadPriority`, ...) with error `code`.
    pub fn failing(call: &'static str, code: u32) -> Self {
        Self {
            fail: Some((call, code)),
            ..Self::default()
        }
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.borrow().clone()
    }

    fn record(&self, name: &'static str, call: Call) -> Result<(), SchedError> {
        self.calls.borrow_mut().push(call);
        match self.fail {
            Some((n, code)) if n == name => Err(SchedError::Os { call: name, code }),
            _ => Ok(()),
        }
    }
}

impl SchedBackend for FakeBackend {
    fn logical_processors(&self) -> usize {
        self.logical
    }
    fn set_priority_class(&self, c: PriorityClass) -> Result<(), SchedError> {
        self.record("SetPriorityClass", Call::Class(c))
    }
    fn set_thread_priority(&self, p: ThreadPriority) -> Result<(), SchedError> {
        self.record("SetThreadPriority", Call::Priority(p))
    }
    fn join_mmcss(&self, task: &str) -> Result<(), SchedError> {
        self.record("AvSetMmThreadCharacteristicsW", Call::Mmcss(task.into()))
    }
    fn pin_current_thread(&self, cpu: usize, mode: PinMode) -> Result<(), SchedError> {
        self.record("SetThreadAffinityMask", Call::Pin(cpu, mode))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_fails_only_the_named_call() {
        let b = FakeBackend::failing("SetPriorityClass", 9);
        assert_eq!(
            b.set_priority_class(PriorityClass::High),
            Err(SchedError::Os {
                call: "SetPriorityClass",
                code: 9
            })
        );
        assert_eq!(b.set_thread_priority(ThreadPriority::Highest), Ok(()));
        assert_eq!(b.calls().len(), 2);
        assert_eq!(b.logical_processors(), 0);
    }
}
