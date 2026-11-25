//! Every topic, declared once. The name, message type and pattern travel together, so a wrong name
//! or type is a compile error. Deployment details (the robot namespace, device instance names) come
//! from the setup file at runtime.

use std::marker::PhantomData;

use crate::msgs::{ArmState, Plan, Status};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    /// Each subscriber sees only the newest value; a late subscriber gets the last one.
    Latest,
    /// Each subscriber sees every value in order, up to `depth` unread (then the oldest drops).
    Queue { depth: usize },
}

#[derive(Debug)]
pub struct Topic<T> {
    pub name: &'static str,
    pub pattern: Pattern,
    _msg: PhantomData<fn() -> T>,
}

impl<T> Topic<T> {
    pub const fn latest(name: &'static str) -> Self {
        Self {
            name,
            pattern: Pattern::Latest,
            _msg: PhantomData,
        }
    }

    pub const fn queue(name: &'static str, depth: usize) -> Self {
        assert!(depth > 0);
        Self {
            name,
            pattern: Pattern::Queue { depth },
            _msg: PhantomData,
        }
    }
}

/// Action chunks: policy → robotd's RT thread.
pub const PLAN: Topic<Plan> = Topic::latest("plan");
/// Robot state every cycle: RT thread → policies, loggers, viewers.
pub const STATE: Topic<ArmState> = Topic::latest("state");
/// Cycle timing and mode, ~10 Hz: RT thread → supervisor, `robotctl`.
pub const STATUS: Topic<Status> = Topic::latest("status");
