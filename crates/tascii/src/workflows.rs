//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use std::{
    collections::hash_map::DefaultHasher,
    hash::Hasher,
    marker::PhantomData,
    panic::RefUnwindSafe,
    sync::Arc,
    //sync::Mutex,
};

use dal::ID;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tracing::debug;

use crate::{runtime::Runtime, task_trait::AsyncRunnable};

#[derive(Serialize, Deserialize, Clone)]
pub enum TaskError {
    /// Task panicked during execution,
    /// the panic was caught by the runtime and
    /// the message from the panic is contained within
    Panic(String),

    /// Task returned Err(T), so T
    /// has been serialized to a string
    Reason(String),

    /// If the runtime experiences an internal error,
    /// Internal can be dispatched. It uses a static string
    /// so that in cases where allocation could panic,
    /// the runtime doesn't have to
    Internal(InternalError),
}

impl std::fmt::Debug for TaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskError::Internal(i) => i.fmt(f),
            TaskError::Panic(p) => {
                writeln!(f, "Panic: {p}")
            }
            TaskError::Reason(r) => {
                writeln!(f, "Reason: {r}")
            }
        }
    }
}

impl TaskError {
    pub fn internal(s: &'static str) -> Self {
        Self::Internal(InternalError { internal: Some(s) })
    }
}

impl From<anyhow::Error> for TaskError {
    fn from(_value: anyhow::Error) -> Self {
        todo!()
    }
}

#[derive(Clone, Debug)]
pub struct InternalError {
    internal: Option<&'static str>,
}

impl Serialize for InternalError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.internal.map(|o| o.to_owned()).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for InternalError {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let v = Option::<String>::deserialize(deserializer)?;
        Ok(Self {
            internal: v.map(String::leak).map(|e| &*e),
        })
    }
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
enum LogEnt {
    Spawn { hash: u64, tid: ID },
}

#[derive(Serialize, Deserialize)]
pub struct ContextInner {
    log: Vec<LogEnt>,
    current_index: usize,

    #[serde(default, skip_serializing, skip_deserializing)]
    rt: Option<&'static Runtime>,

    tid: ID, // the Uuid of the task this is in
}

impl std::fmt::Debug for ContextInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextInner")
            .field("log", &self.log)
            .field("current_index", &self.current_index)
            .field("tid", &self.tid)
            .finish()
    }
}

#[derive(Debug)]
pub struct Context {
    pub inner: Arc<Mutex<ContextInner>>,
}

impl<'de> Deserialize<'de> for Context {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = Self {
            inner: Arc::new(Mutex::new(ContextInner::deserialize(deserializer)?)),
        };

        debug!("Deserializes, inner for context: {:?}", s.inner.lock());

        Ok(s)
    }
}

impl Serialize for Context {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let inner = self.inner.lock();

        debug!("Serializing context, it has an inner of {:?}", inner);
        inner.serialize(serializer)
    }
}

impl Context {
    pub fn within(rt: &'static Runtime, tid: ID) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ContextInner {
                log: vec![],
                current_index: 0,
                rt: Some(rt),
                tid,
            })),
        }
    }

    /// If no prior run with this context existed, this
    /// creates a new runtime task from the provided runnable
    /// and returns a handle that can be joined on to block until
    /// that task finishes
    ///
    /// If recovering from a partial run, joining
    /// that handle will instead load the result from
    /// the prior run from disk and return that
    pub fn spawn<R, D>(&self, t: R) -> CtxJoinHandle<D>
    where
        R: AsyncRunnable<Output = D> + 'static,
        D: 'static + Send,
    {
        let mut inner = self.inner.lock();

        let tid = inner.tid;

        let mut hasher = DefaultHasher::new();
        t.hash(&mut hasher);

        let ghash = hasher.finish();

        // there was a prior execution that made it "this far", so try to realign and recover
        if let Some(&v) = inner.log.get(inner.current_index) {
            debug!("there was an entry! Ghash was {ghash}");

            match v {
                // the hash of what we're trying to spawn matched the existing hash,
                // so we can take the result of the prior run and shortcut-return
                // that instead
                LogEnt::Spawn { hash, tid } if hash == ghash => {
                    // return the existing task
                    let jh = CtxJoinHandle {
                        ctx: self.clone_context_ref(),
                        tid,
                        _p: PhantomData,
                    };

                    inner.current_index += 1;

                    return jh;
                }
                // it didn't match, so we should truncate our log, notify
                // of an error, and continue with newly created tasks
                // (we have broken from our prior execution flow--eek!)
                LogEnt::Spawn { hash, tid } => {
                    tracing::error!(
                        "Hash was {hash} but ghash was {ghash}, task is {tid}, so log was misaligned. Truncating log and starting back"
                    );
                    let ci = inner.current_index;
                    inner.log.truncate(ci);
                }
            }
        }

        debug!("there was no entry to match against, or the log needed to be realigned");

        let id = inner
            .rt
            .expect("there was no runtime for context")
            .enroll(t.into());

        debug!("(task {tid}) Enrolled the task by id {id}, spawned into the runtime");

        inner.rt.unwrap().set_target(id);

        let jh = CtxJoinHandle {
            ctx: self.clone_context_ref(),
            tid: id,
            _p: PhantomData,
        };

        // create a new task and contract since we didn't have one to recover
        // put task into our log as an entry
        // return the handle

        inner.log.push(LogEnt::Spawn {
            hash: ghash,
            tid: id,
        });

        inner.current_index += 1;

        let rt = inner.rt.unwrap();

        std::mem::drop(inner);

        debug!("(task {tid}) Finished starting new task, exiting from spawn...");

        // make sure since we're updating our internal ticker
        // that we commit our task so that is noted and saved
        rt.with_task(tid, |t| t.commit())
            .expect("couldn't get current task")
            .expect("couldn't commit current task");

        jh
    }

    fn clone_context_ref(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }

    fn with_inner<T, F>(&self, f: F) -> T
    where
        F: FnOnce(&mut ContextInner) -> T,
    {
        let mut g = self.inner.lock();

        f(&mut g)
    }

    /// Blocks until the referenced task completes (with a result or in error)
    /// Should not be called from anything except a JoinHandle
    #[doc(hidden)]
    fn join<
        D: Sized
            + Clone
            + Send
            + Sync
            + RefUnwindSafe
            + Serialize
            + DeserializeOwned
            + 'static
            + std::fmt::Debug,
    >(
        &self,
        id: ID,
    ) -> Result<D, TaskError> {
        self.join_inner(id)
    }

    /// Yields until the referenced task completes (with a result or in error)
    /// Should not be called externally
    #[doc(hidden)]
    fn join_inner<
        D: Sized
            + Clone
            + Send
            + Sync
            + RefUnwindSafe
            + Serialize
            + DeserializeOwned
            + 'static
            + std::fmt::Debug,
    >(
        &self,
        id: ID,
    ) -> Result<D, TaskError> {
        self.with_inner(|inner| {
            debug!(
                "(task {}) join called against {id}, waiting for it to complete",
                inner.tid
            )
        });

        let rt = self
            .inner
            .lock()
            .rt
            .ok_or(TaskError::internal("a context inner had no runtime"))?;

        debug!("Got inner, going to hold it across a with_task and wait");
        let tr: Result<D, TaskError> = rt
            .with_task(id, |t| {
                debug!("Cloning result");
                // first treat the result as one that returns a TaskReturnVal of *D*
                let r = t.result.clone();

                debug!("going to wait against task {id} for finish result because of join");

                r
            })
            .map_err(|_e| TaskError::internal("couldn't get the task that was being joined on"))?
            .to_typed::<Result<D, TaskError>>()
            .expect("Invariant violated: bad join type downcast")
            .wait()
            .expect("wait for a oneshot failed, bad recv?");

        self.with_inner(|inner| debug!("(task {}) join returns from waiting on {id} after it completed. It returned {tr:?}", inner.tid));

        tr
    }

    /// Waits until all tasks in question have finished,
    /// returns a collection of the results of those tasks
    fn join_all<
        D: Sized
            + Send
            + Sync
            + Clone
            + RefUnwindSafe
            + Serialize
            + DeserializeOwned
            + 'static
            + std::fmt::Debug,
        I: IntoIterator<Item = CtxJoinHandle<D>>,
    >(
        &self,
        handles: I,
    ) -> Vec<Result<D, TaskError>> {
        handles.into_iter().map(|e| e.join()).collect()
    }

    /// set_volatile tells this context to wipe its log,
    /// saying "if we are rerunning this task, we need
    /// to start from the beginning and ignore any partial state"
    ///
    /// Use this for tasks that should be done roughly atomically,
    /// such as anything time sensitive or where having an arbitrary delay
    /// inserted "anywhere" in the code would have bad effects.
    ///
    /// Examples include opening a session that expires, or configuring a host
    /// boot options where it would be easy for the host rebooting to
    /// cause that config to be lost
    pub fn set_volatile(&self) {
        self.inner.lock().log.truncate(0); // empty the log
    }

    pub fn set_runtime(&self, rt: &'static Runtime) {
        debug!("sets runtime");
        self.inner.lock().rt = Some(rt);
        debug!("done set runtime");
    }

    pub fn reset(&self) {
        self.with_inner(|i| i.current_index = 0);
    }
}

pub struct CtxJoinHandle<D> {
    tid: ID,
    ctx: Context,
    _p: PhantomData<D>,
}

impl<
    D: Sized
        + Send
        + Sync
        + Clone
        + RefUnwindSafe
        + Serialize
        + DeserializeOwned
        + 'static
        + std::fmt::Debug,
> CtxJoinHandle<D>
{
    pub fn join(&self) -> Result<D, TaskError> {
        self.ctx.join::<D>(self.tid)
    }
}

// try to tell users that they shouldn't allow nondeterminism
//impl !Send for Context {}
//impl !Sync for Context {}
