//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use std::{any::type_name, hash::Hash, panic::RefUnwindSafe, time::Duration};

use dal::ID;

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::{
    task_shim::RunnableHandle,
    workflows::{Context, TaskError},
};

/// A non-object-safe trait that simplifies implementing DynRunnable
pub trait Runnable:
    Send
    + Clone
    + Sized
    + std::fmt::Debug
    + Hash
    + Sync
    + Serialize
    + DeserializeOwned
    + TaskRegistered
    + 'static
    + RefUnwindSafe
{
    type Output: TaskSafe;

    fn execute_task(&mut self, context: &Context) -> Result<Self::Output, TaskError>;

    /// Called to "run" this task, returns on completion of the
    /// task (or failure)
    ///
    /// `context` provides inversion-of-control style runtime
    /// facilities to the task, including for task spawning and similar
    fn run(&mut self, context: &Context) -> Result<Self::Output, TaskError>;

    /// Provided with the id of the wrapping task
    ///
    /// Generally shouldn't be overridden as it exists here
    fn summarize(&self, id: ID) -> String {
        let task_ty_name = type_name::<Self>();
        format!("Task {task_ty_name} with ID {id}")
    }

    fn timeout() -> Duration {
        Duration::from_secs_f64(600.0)
    }

    /// How many times the runtime should retry running the task on failure
    /// before declaring it "failed" and continuing within the runtime
    ///
    /// The default is zero retries after a first try
    fn retry_count() -> usize {
        0
    }

    fn identifier() -> TaskIdentifier;
}

/// Trait used for creating "Tascii tasks".
pub trait AsyncRunnable:
    Send
    + Clone
    + Sized
    + std::fmt::Debug
    + Hash
    + Sync
    + Serialize
    + DeserializeOwned
    + TaskRegistered
    + 'static
    + RefUnwindSafe
{
    type Output: TaskSafe;

    // Actual body of the task. Called by the run function.
    async fn execute_task(&mut self, context: &Context) -> Result<Self::Output, TaskError>;

    // Called by the tascii scheduling / task management system. Executes the task body while accounting for retries and timeouts.
    // DO NOT modify this implementation in trait implementations unless you want to entirely circumvent the timeout / retry mechansism.
    async fn run(&mut self, context: &Context) -> Result<Self::Output, TaskError> {

        let mut last_error = TaskError::Reason("Task never ran".to_owned());

        for attempt_number in 0..(Self::retry_count() + 1) {
            tracing::info!("Running task {self:?} (attempt no. {attempt_number})");
            let timeout_result = tokio::time::timeout(Self::timeout(), self.execute_task(context)).await;

            match timeout_result {
                Ok(task_result) => match task_result {
                    Ok(output) => return Ok(output),
                    Err(error) => last_error = error
                },
                Err(_) => {
                    last_error = TaskError::Reason("Task Timed out".to_owned());
                    tracing::info!("Task {self:?} timed out!");
                }
            }

            std::thread::sleep(Self::retry_buffer_time());
        }

        Err(last_error)
    }

    /// Provided with the id of the wrapping task
    fn summarize(&self, id: ID) -> String {
        let task_ty_name = type_name::<Self>();
        format!("Async Task {task_ty_name} with ID {id}")
    }

    /// The timeout duration for each task attempt (not total duration)
    /// Upon timeout, the task run will exit and be treated as a failed attempt, triggering the retry mechanism.
    /// Should be configured on per-task basis to some reasonable number.
    /// It is recommended to take into account the overall timeout of child tasks when choosing each task's timeout.
    /// When in doubt, give a task more time than you'd expect it to take to complete.
    /// If you have any cycles in your task call structure and attempt to configure this timeout using overall timeouts,
    /// you will recurse infinitely and get a stack overflow (example: Task A calls Task B and Task B calls Task A) 
    fn timeout() -> Duration;


    /// The total timeout including retries and buffer time.
    /// There is no need to implement this in trait implementations
    fn overall_timeout() -> Duration {
        (Self::timeout() + Self::retry_buffer_time()) * (Self::retry_count() as u32 + 1)
    }

    /// How many times the runtime should retry running the task on failure
    /// before declaring it "failed" and continuing within the runtime.
    /// The task will run at least once, even if retry count is 0.
    fn retry_count() -> usize {
        0
    }

    /// Somewhat arbitrary time to wait in between retry attempts
    /// Can be useful to prevent issues caused from tasks retrying too quicky (e.g. IPMI request attempts)
    fn retry_buffer_time() -> Duration {
        Duration::from_secs(5)
    }

    fn identifier() -> TaskIdentifier;
}

impl<T> AsyncRunnable for T
where
    T: Runnable,
{
    type Output = <Self as Runnable>::Output;

    async fn execute_task(&mut self, context: &Context) -> Result<Self::Output, TaskError> {
        self.execute_task(context)
    }

    async fn run(&mut self, context: &Context) -> Result<Self::Output, TaskError> {
        self.run(context)
    }

    fn identifier() -> TaskIdentifier {
        Self::identifier()
    }

    fn summarize(&self, id: ID) -> String {
        self.summarize(id)
    }

    fn timeout() -> Duration {
        <Self as Runnable>::timeout()
    }

    fn retry_count() -> usize {
        Self::retry_count()
    }
}

pub trait TaskSafe:
    std::fmt::Debug + Send + Sync + Serialize + DeserializeOwned + 'static + Clone + RefUnwindSafe
{
}

impl<
        T: std::fmt::Debug
            + Send
            + Sync
            + Serialize
            + DeserializeOwned
            + 'static
            + Clone
            + RefUnwindSafe,
    > TaskSafe for T
{
}

#[derive(PartialEq, Eq, Hash, Clone, Serialize, Deserialize, Debug)]
pub struct TaskIdentifier {
    version: usize,
    name: String,
}

impl TaskIdentifier {
    pub fn versioned(self, version: usize) -> Self {
        Self { version, ..self }
    }

    pub fn named(name: &'static str) -> Self {
        Self {
            name: name.into(),
            version: 1,
        }
    }
}

#[derive(Clone)]
pub struct TaskMarker {
    pub(crate) deserialize_fn: fn(serde_json::Value) -> RunnableHandle,
    pub(crate) serialize_fn: fn(&RunnableHandle) -> Box<serde_json::Value>,
    pub(crate) ident: fn() -> TaskIdentifier,
}

unsafe impl Send for TaskMarker {}
unsafe impl Sync for TaskMarker {}

macro_reexport::collect!(TaskMarker);

/// A marker trait indicating you should tack on a `tascii::mark_task!(<your task type>)`
/// before your task
#[allow(clippy::missing_safety_doc)]
pub unsafe trait TaskRegistered {}

pub mod macro_reexport {
    pub use inventory::*;
}

pub use crate::task_shim::register_task;

#[macro_export]
macro_rules! mark_task {
    ($task:ty) => {
        tascii::task_trait::macro_reexport::submit! { unsafe { tascii::task_trait::register_task::<$task>() } }

        unsafe impl tascii::task_trait::TaskRegistered for $task {}
    };
}
