//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use std::{hash::Hash, panic::RefUnwindSafe, time::Duration, any::type_name};

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

    /// The duration from task start until the runtime should declare
    /// it as "hung" if it has not yet finished
    ///
    /// Runtime will terminate the task and do retry/failure next action
    fn variable_timeout(&self) -> Duration {
        Self::timeout()
    }

    fn timeout() -> Duration {
        Duration::from_secs_f64(600.0)
    }

    /// How many times the runtime should retry running the task on failure
    /// before declaring it "failed" and continuing within the runtime
    ///
    /// The default is zero retries after a first try
    fn retry_count(&self) -> usize {
        0
    }

    fn identifier() -> TaskIdentifier;
}

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

    async fn run(&mut self, context: &Context) -> Result<Self::Output, TaskError>;

    /// Provided with the id of the wrapping task
    fn summarize(&self, id: ID) -> String {
        let task_ty_name = type_name::<Self>();
        format!("Async Task {task_ty_name} with ID {id}")
    }

    /// The duration from task start until the runtime should declare
    /// it as "hung" if it has not yet finished
    ///
    /// Runtime will terminate the task and do retry/failure next action
    fn variable_timeout(&self) -> Duration {
        Self::timeout()
    }

    fn timeout() -> Duration {
        Duration::from_secs_f64(120.0)
    }

    /// How many times the runtime should retry running the task on failure
    /// before declaring it "failed" and continuing within the runtime
    ///
    /// The default is zero retries after a first try
    fn retry_count(&self) -> usize {
        0
    }

    fn identifier() -> TaskIdentifier;
}

impl<T> AsyncRunnable for T
where T: Runnable
{
    type Output = <Self as Runnable>::Output;

    async fn run(&mut self, context: &Context) -> Result<Self::Output, TaskError> {
        let r = self.run(&context);

        r
    }

    fn identifier() -> TaskIdentifier {
        Self::identifier()
    }

    fn summarize(&self, id: ID) -> String {
        self.summarize(id)
    }

    fn variable_timeout(&self) -> Duration {
        self.variable_timeout()
    }

    fn timeout() -> Duration {
        <Self as Runnable>::timeout()
    }

    fn retry_count(&self) -> usize {
        self.retry_count()
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
    pub(crate) deserialize_fn: fn(Box<serde_json::Value>) -> RunnableHandle,
    pub(crate) serialize_fn: fn(&RunnableHandle) -> Box<serde_json::Value>,
    pub(crate) ident: fn() -> TaskIdentifier,
}

unsafe impl Send for TaskMarker {}
unsafe impl Sync for TaskMarker {}

macro_reexport::collect!(TaskMarker);

/// A marker trait indicating you should tack on a `tascii::mark_task!(<your task type>)`
/// before your task
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
