//! Deterministic dependency scheduling in stable parallel batches.

mod scheduler;

pub use scheduler::{ScheduleError, Task, schedule_batches};
