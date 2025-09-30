//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use std::{
    collections::HashSet,
    fmt::Debug,
    sync::{atomic::compiler_fence, Arc},
};

use dal::{web::AnyWaySpecStr, AsEasyTransaction, DBTable, FKey, Row, SchrodingerRow, ID};
use itertools::Itertools;
use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use serde::{Deserialize, Serialize};

#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

use crate::{
    executors,
    oneshot::{SimpleOneshotHandle, StrongUntypedOneshotHandle},
    runtime::Runtime,
    task_shim::RunnableHandle,
    workflows::{Context, TaskError},
};

#[derive(Debug, Clone)]
pub struct RuntimeTask {
    id: FKey<RuntimeTask>,

    /// proto is the task prototype from which instances are
    /// derived to run
    pub proto: RunnableHandle,

    pub result: StrongUntypedOneshotHandle,

    pub context: Arc<Context>,

    /// The set of tasks that need to reach the Complete metastate
    /// before this task can run
    pub depends_on: HashSet<ID>,

    /// The set of tasks that have not yet completed that
    /// this task is waiting to have reach the Complete metastate
    pub waiting_for: HashSet<ID>,

    /// The set of tasks that should be looked at to potentially
    /// run if this task completes
    pub depends_for: HashSet<ID>,
}

impl DBTable for RuntimeTask {
    fn table_name() -> &'static str {
        "tascii_runtime_tasks"
    }

    fn id(&self) -> dal::ID {
        self.id.into_id()
    }

    fn id_mut(&mut self) -> &mut dal::ID {
        self.id.into_id_mut()
    }

    fn from_row(row: Row) -> Result<dal::ExistingRow<Self>, anyhow::Error> {
        let task: RunnableHandle = serde_json::from_value(row.try_get("proto")?)?;

        let simple_result: SimpleOneshotHandle = serde_json::from_value(row.try_get("result")?)?;

        let result = task.task_ref().unmarshal(simple_result);

        Ok(dal::ExistingRow::from_existing(Self {
            id: row.try_get("id")?,
            proto: task,
            result,
            context: serde_json::from_value(row.try_get("context")?)?,
            depends_on: serde_json::from_value(row.try_get("depends_on")?)?,
            waiting_for: serde_json::from_value(row.try_get("waiting_on")?)?,
            depends_for: serde_json::from_value(row.try_get("depends_for")?)?,
        }))
    }

    fn to_rowlike(
        &self,
    ) -> Result<std::collections::HashMap<&str, Box<dyn dal::ToSqlObject>>, anyhow::Error> {
        let task = serde_json::to_value(&self.proto)
            .map_err(|e| format!("err: {e:?}"))
            .anyway()?;

        let result = serde_json::to_value(self.result.simplify())?;

        let context = serde_json::to_value(&*self.context)?;

        let depends_on = self.depends_on.clone().into_iter().collect_vec();

        let waiting_for = self.waiting_for.clone().into_iter().collect_vec();

        let depends_for = self.depends_for.clone().into_iter().collect_vec();

        let state = serde_json::to_value(self.proto.task_ref().status(&self.result))?;

        let r = Ok(vec![
            dal::col("id", self.id),
            dal::col("proto", task),
            dal::col("result", result),
            dal::col("context", context),
            dal::col("depends_on", depends_on),
            dal::col("waiting_for", waiting_for),
            dal::col("depends_for", depends_for),
            dal::col("state", state),
        ]
        .into_iter()
        .collect());

        tracing::trace!("Task serializes as: {r:?}");

        r
    }
}

impl RuntimeTask {
    /// This is the only canonical constructor for a RuntimeTask,
    /// since it needs to save the task off to the database
    /// and make sure that following calls to commit()
    /// can succeed
    pub(crate) fn build(
        proto: RunnableHandle,
        within_rt: &'static Runtime,
    ) -> Result<Self, anyhow::Error> {
        let id = FKey::new_id_dangling();

        let oneshot = proto.task_ref().oneshot()?;

        Ok(RuntimeTask {
            proto,

            id,

            context: Arc::new(Context::within(within_rt, id.into_id())),
            depends_for: HashSet::new(),
            depends_on: HashSet::new(),
            waiting_for: HashSet::new(),
            result: oneshot,
        })
    }

    pub(crate) fn cancel(&self, why: TaskError) -> Result<(), anyhow::Error> {
        let res = self.proto.task_ref().on_error()(self.result.clone(), why);

        res.map_err(|_| anyhow::Error::msg("task was already completed"))
    }

    pub(crate) fn status(&self) -> TaskState {
        self.proto.task_ref().status(&self.result)
    }

    pub(crate) fn is_complete(&self) -> bool {
        let s = self.status();

        match s {
            TaskState::Done => true,
            TaskState::Failed => true,
            TaskState::Ready => false,
        }
    }

    pub(crate) fn run(self_id: ID, rt: &'static Runtime) {
        // set up the context
        tracing::debug!(
            "entry of RuntimeTask, if see this but not next print then clone probably panicked"
        );

        let (mut proto, result, pre_context) = rt
            .with_task(self_id, |t| {
                (t.proto.clone(), t.result.clone(), t.context.clone())
            })
            .expect("couldn't get task info");

        let proto_summary = proto.task().summarize(self_id);
        let proto_ident = proto.task().identifier();
        tracing::debug!("starts run of task {self_id} in RuntimeTask, summary: {proto_summary}, ident: {proto_ident:?}");

        compiler_fence(std::sync::atomic::Ordering::SeqCst);

        debug!("cloned result, about to run task within catch_unwind");

        let proto_summary = proto.task().summarize(self_id);
        let proto_ident = proto.task().identifier();

        let task_closure = move || {
            let task = proto.task();

            debug!("got the task from proto");

            // we can run the task here since we're inside an unwind catch
            debug!("starts run of task {self_id}");
            task.run(rt, result, self_id, pre_context)
        };

        // we try to trust the task closure to complete without panicking,
        // but we catch again to make sure the runtime can never fall over
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            debug!("running task with caught panics");
            task_closure()
        }))
        .expect("very bad panic, TODO fix this");

        tracing::debug!("task {self_id} finished execution, in some way. This was a task of kind {proto_ident:?}, summarizes as {proto_summary}");
    }
}

impl RuntimeTask {
    pub(crate) fn summarize(&self) -> String {
        format!(
            "{:width$} {:width2$}",
            format!("{:?}", self.status()),
            self.proto.task_ref().summarize(self.id()),
            width = 10,
            width2 = 100
        )
    }

    pub(crate) async fn commit_async(&self) -> Result<(), anyhow::Error> {
        let tid = self.id.into_id();
        debug!("Commits task {tid}");

        // we insert ourselves if we didn't already exist, saving off to db
        let sr: SchrodingerRow<RuntimeTask> = SchrodingerRow::new(self.clone());

        let mut client = dal::new_client().await?;
        let mut trans = client.easy_transaction().await?;

        let summary = self.proto.task_ref().summarize(self.id());
        tracing::debug!("Committing task {summary}");

        sr.upsert(&mut trans).await?;

        trans.commit().await?;

        Ok(())
    }

    pub(crate) fn commit(&self) -> Result<(), anyhow::Error> {
        // The spawn_on_tascii_tokio_primary is intentionally designed to catch panics
        // and to abort the program if that occurs, since we consider
        // things happening on the primary tokio runtime to be "critical"
        //
        // Thus, this is sound even with how horrible it first appears
        //
        // for now, we're just cloning things to try to avoid some
        // strange hole in my logic from before
        let sc = self.clone();
        executors::spawn_on_tascii_tokio("task_commit", async move { sc.commit_async().await })?;

        Ok(())
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
pub enum TaskState {
    Ready,
    Failed,
    Done,
}

// We can introduce a guarded locking mechanism that is basically a TaskGuard<'static, Task> that
// derefs to Task but, when dropped for a task in the Done state, allows unloading the task from
// memory
//
// We can put this behind a get_task() function that takes an ID, that can load the corresponding
// tasks from the db
//
// This guard should have both acquire_mut and acquire_ref methods that both refer to the same task
// in memory. The guard should also have a commit() method that saves the task off to db

#[derive(Clone, Debug)]
pub struct TaskGuard {
    pub inner: Arc<TaskGuardInner>,
}

#[derive(Debug)]
pub struct TaskGuardInner {
    pub task: RwLock<RuntimeTask>,
}

impl std::ops::Deref for TaskGuard {
    type Target = TaskGuardInner;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref()
    }
}

impl TaskGuardInner {
    pub fn get_ref(&self) -> Result<RwLockReadGuard<'_, RuntimeTask>, anyhow::Error> {
        Ok(self.task.read())
    }

    pub fn get_mut(&self) -> Result<RwLockWriteGuard<'_, RuntimeTask>, anyhow::Error> {
        Ok(self.task.write())
    }

    pub fn commit(&self) -> Result<(), anyhow::Error> {
        let task = self.get_ref()?;
        task.commit()
    }
}

// we can remove ourselves from the registry of all tasks if we are in a Done state
// (this saves us memory, and is transparent to the user if they need to check for us again)
//
// NOTE: this was considered, but should not be done because of a logic issue
// where a task that has just been created and added can drop itself
//
// Instead, we let Weak go dead and get_task handles any re-load
//
// TODO: add a thread that comes by every day or so that checks for dead Weaks and just removes
// them from the map
impl Drop for TaskGuardInner {
    fn drop(&mut self) {
        //debug!("task guard drops, the rc will go dead for cache");
        // ALL_TASKS.lock().unwrap().remove(&self.get_ref().id);

        // now we drop, freeing any memory from the task automatically
    }
}
