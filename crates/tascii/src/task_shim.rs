//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use std::{
    any::type_name, collections::HashMap, panic::AssertUnwindSafe, sync::OnceLock, time::Duration,
};

use dal::ID;

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info};

use crate::{
    executors,
    oneshot::{OneShot, OneShotRegistry, SimpleOneshotHandle, StrongUntypedOneshotHandle},
    runtime::Runtime,
    scheduler,
    task_runtime::TaskState,
    task_trait::{AsyncRunnable, TaskIdentifier, TaskMarker, TaskSafe},
    workflows::{Context, TaskError},
};

#[derive(Debug)]
pub struct DynRunnableShim<R: AsyncRunnable> {
    v: R,
}

/// DO NOT implement this trait yourself, we rely on the below impl
/// being effectively sealed, so we can do some pointer magic
///
/// DO NOT call any method here from an async context heirarchically
/// below dyntask
pub(crate) trait DynRunnable: Send + std::fmt::Debug + Sync {
    fn run(
        &mut self,
        rt: &'static Runtime,
        oneshot: StrongUntypedOneshotHandle,
        run_id: ID,
        context: Arc<Context>,
    );

    fn status(&self, with_result: &StrongUntypedOneshotHandle) -> TaskState;

    /// Provided with the id of the wrapping task
    fn summarize(&self, id: ID) -> String;

    /// The duration from task start until the runtime should declare
    /// it as "hung" if it has not yet finished
    ///
    /// Runtime will terminate the task and do retry/failure next action
    fn timeout(&self) -> Duration;

    fn oneshot(&self) -> Result<StrongUntypedOneshotHandle, anyhow::Error>;

    fn unmarshal(&self, h: SimpleOneshotHandle) -> StrongUntypedOneshotHandle;

    fn complete_with(
        &self,
        oneshot: &StrongUntypedOneshotHandle,
        value: Result<String, TaskError>,
    ) -> Result<(), anyhow::Error>;

    /// Ok if the task was not yet complete and this completed it,
    /// Err if the task was already completed
    fn on_error(&self) -> Box<dyn Fn(StrongUntypedOneshotHandle, TaskError) -> Result<(), ()>>;

    fn clone(&self) -> Box<dyn DynRunnable>;

    fn to_value(&self) -> Box<serde_json::Value>;

    fn identifier(&self) -> TaskIdentifier;

    fn fake_success(
        &self,
        v: serde_json::Value,
        h: StrongUntypedOneshotHandle,
    ) -> Result<(), anyhow::Error>;
}

impl<R> DynRunnable for DynRunnableShim<R>
where
    R: AsyncRunnable,
{
    fn run(
        &mut self,
        rt: &'static Runtime,
        oneshot: StrongUntypedOneshotHandle,
        run_id: ID,
        context: Arc<Context>,
    ) {
        // just make sure we have our env correct going in
        context.set_runtime(rt);

        let tokio_rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("couldn't init a tokio runtime for the task");

        // now, run the task
        debug!("DynRunnable run for task {run_id}");

        context.reset();

        // force timeout to happen here,
        // when we have the context of what the result type is
        let timeout_oneshot = oneshot.clone();
        let timeout = self.timeout();

        debug!("Got timeout and oneshot, going to spawn the timeout task into the runtime, task {run_id}");

        let summary = self.summarize(run_id);
        let th = oneshot.clone();
        // make it so we get a feedback print when this task finishes
        std::thread::spawn(move || {
            let th = th.to_typed::<Result<R::Output, TaskError>>().unwrap();

            let res = th.wait().unwrap();

            info!("Task {summary} completed, result was {res:?}");
        });

        // set a timeout for the task so we don't continue blocking forever, and
        // we get isolated and cut off if we run too long
        executors::spawn_on_tascii_tokio("oneshot", async move {
            tokio::spawn(async move {
                tokio::time::sleep(timeout).await;
                let oneshot = timeout_oneshot;
                let typed: &OneShot<Result<R::Output, TaskError>> = oneshot
                    .to_typed()
                    .expect("TASCII violated an invariant, gave us a oneshot we didn't give it");

                let timeout_res = typed.complete_with(Err(TaskError::Timeout())).await;

                if timeout_res {
                    rt.send(scheduler::TaskMessage::Timeout(run_id))
                        .expect("couldn't time out a task")
                }
            });
        });

        debug!("Set the timeout, about to block on finishing the task by id {run_id}");

        let slef: &'static mut Self = unsafe { std::mem::transmute(self) };

        let res = std::panic::catch_unwind(AssertUnwindSafe(|| {
            tokio_rt.block_on(async move {
                let r = slef.v.run(&context).await;

                let _summary = slef.summarize(run_id);

                debug!("Going to complete the oneshot");
                // this already internally saves any changes,

                debug!("Completed the oneshot");

                r

                // TODO: we want to atomically notify the scheduler that we've completed
            })
        }));

        let complete_with = match res {
            Err(_e) => Err(TaskError::Panic("Task failed to run, panicked".to_string())),
            Ok(Err(e)) => Err(TaskError::Reason(format!(
                "Task failed to run, reason: {e:?}"
            ))),
            Ok(Ok(v)) => Ok(v),
        };

        executors::spawn_on_tascii_tokio("oneshot", async move {
            oneshot
            .to_typed()
            .expect(
                "TASCII provided us with an incorrectly typed oneshot, not the one we sent back?",
            )
            .complete_with(complete_with.clone()) // TODO: clone is their code,
                                                  // so we can't know it won't panic
            .await;

            debug!(
                "Task {} has completed its oneshot, with result: {:?}",
                run_id, complete_with
            );

            debug!("Task {run_id} unblocked and is returning");
        });
    }

    fn on_error(&self) -> Box<dyn Fn(StrongUntypedOneshotHandle, TaskError) -> Result<(), ()>> {
        let _summary = self.summarize(ID::nil());
        fn on_error<Res: TaskSafe>(h: StrongUntypedOneshotHandle, e: TaskError) -> Result<(), ()> {
            executors::spawn_on_tascii_tokio("oneshot", async move {
                match h.to_typed::<Result<Res, TaskError>>() {
                    Some(o) => match o.complete_with(Err(e)).await {
                        true => Ok(()),
                        false => Err(()),
                    },
                    None => {
                        unreachable!("passed back wrong oneshot type")
                    }
                }
            })
        }

        // fn items are gross and weird, TODO: revisit this
        // I think he is talking about the Box::new() ^^^
        Box::new(on_error::<R::Output>)
    }

    fn fake_success(
        &self,
        v: serde_json::Value,
        h: StrongUntypedOneshotHandle,
    ) -> Result<(), anyhow::Error> {
        let v: R::Output = serde_json::from_value(v)?;

        executors::spawn_on_tascii_tokio("oneshot", async move {
            match h.to_typed::<Result<R::Output, TaskError>>() {
                Some(o) => match o.complete_with(Ok(v)).await {
                    true => Ok(()),
                    false => Err(anyhow::Error::msg("task was already completed before now")),
                },
                None => {
                    unreachable!("passed back wrong oneshot type")
                }
            }
        })
    }

    fn unmarshal(&self, h: SimpleOneshotHandle) -> StrongUntypedOneshotHandle {
        tracing::debug!("Unmarshalls a task of type {}", self.summarize(ID::nil()));
        executors::spawn_on_tascii_tokio("oneshot", async move {
            OneShotRegistry::get_as_any::<Result<R::Output, TaskError>>(h.id).await
        })
    }

    fn status(&self, with_result: &StrongUntypedOneshotHandle) -> TaskState {
        let os = with_result
            .to_typed::<Result<R::Output, TaskError>>()
            .expect("TASCII invariant violated: incorrect oneshot given to task");

        // TODO: this part is not particularly panic safe, even though
        // we run drop code for the value!
        match os.get() {
            Some(Ok(_v)) => TaskState::Done,
            Some(Err(_e)) => TaskState::Failed,
            None => TaskState::Ready,
        }
    }

    fn oneshot(&self) -> Result<StrongUntypedOneshotHandle, anyhow::Error> {
        executors::spawn_on_tascii_tokio("oneshot", async {
            OneShotRegistry::new_task_oneshot::<R::Output>().await
        })
    }

    fn summarize(&self, _id: ID) -> String {
        let task_ty_name = type_name::<R>();

        format!("Task {task_ty_name} with content {:#?}", self.v)
    }

    fn timeout(&self) -> Duration {
        self.v.variable_timeout()
    }

    fn identifier(&self) -> TaskIdentifier {
        R::identifier()
    }

    fn clone(&self) -> Box<dyn DynRunnable> {
        Box::new(DynRunnableShim { v: self.v.clone() })
    }

    fn to_value(&self) -> Box<serde_json::Value> {
        Box::new(serde_json::to_value(&self.v).expect("couldn't serialize task"))
    }

    fn complete_with(
        &self,
        oneshot: &StrongUntypedOneshotHandle,
        value: Result<String, TaskError>,
    ) -> Result<(), anyhow::Error> {
        let oneshot = oneshot.clone();
        executors::spawn_on_tascii_tokio("oneshot", async move {
            let os = oneshot
                .to_typed::<Result<R::Output, TaskError>>()
                .expect("bad oneshot type");

            let cr = match value {
                Ok(json) => {
                    let value_parsed: R::Output = serde_json::from_str(&json)?;
                    Ok(value_parsed)
                }
                Err(e) => Err(e),
            };
            let completes = os.complete_with(cr).await;

            if completes {
                Ok(())
            } else {
                Err(anyhow::Error::msg(
                    "oneshot was already completed, couldn't push new completion",
                ))
            }
        })
    }
}

#[derive(Debug)]
pub struct RunnableHandle {
    pub(crate) task: Box<dyn DynRunnable>,
}

#[derive(Serialize, Deserialize)]
pub struct SerializedTask {
    task: serde_json::Value,
    id: TaskIdentifier,
}

impl<'de> Deserialize<'de> for RunnableHandle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let st = SerializedTask::deserialize(deserializer)?;

        let tm = collect_tasks()
            .get(&st.id)
            .cloned()
            .expect("failed to deserialize a task, big bad!");

        let df = tm.deserialize_fn;

        Ok(df(st.task))
    }
}

impl Serialize for RunnableHandle {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let tm = collect_tasks()
            .get(&self.task.identifier())
            .cloned()
            .expect("failed to deserialize a task, big bad!");

        let sf = tm.serialize_fn;

        sf(self).serialize(serializer)
    }
}

impl RunnableHandle {
    pub(crate) fn task(&mut self) -> &mut dyn DynRunnable {
        &mut *self.task
    }

    pub(crate) fn task_ref(&self) -> &dyn DynRunnable {
        &*self.task
    }

    pub(crate) fn new(task: Box<dyn DynRunnable>) -> Self {
        Self { task }
    }
}

impl<R: AsyncRunnable + 'static> From<R> for RunnableHandle {
    fn from(value: R) -> Self {
        Self::new(Box::new(DynRunnableShim { v: value }))
    }
}

impl Clone for RunnableHandle {
    fn clone(&self) -> Self {
        Self {
            task: self.task.clone(),
        }
    }
}

static TASKS: std::sync::OnceLock<HashMap<TaskIdentifier, TaskMarker>> = OnceLock::new();

pub fn collect_tasks() -> &'static HashMap<TaskIdentifier, TaskMarker> {
    TASKS.get_or_init(|| {
        let mut c = HashMap::new();
        for m in inventory::iter::<TaskMarker> {
            //let m = *m;
            let id = m.ident;
            c.insert(id(), m.clone());
        }

        c
    })
}

pub const fn register_task<T: AsyncRunnable + Serialize + DeserializeOwned + 'static>() -> TaskMarker
{
    fn serialize(rh: &RunnableHandle) -> Box<serde_json::Value> {
        let p = &*rh.task;

        p.to_value()
    }

    fn deserialize<T: AsyncRunnable + DeserializeOwned + 'static>(
        val: serde_json::Value,
    ) -> RunnableHandle {
        let t: T = serde_json::from_value(val).expect("couldn't deserialize task");
        let boxed: Box<dyn DynRunnable> = Box::new(DynRunnableShim { v: t });

        RunnableHandle { task: boxed }
    }

    TaskMarker {
        deserialize_fn: deserialize::<T>,
        serialize_fn: serialize,
        ident: T::identifier,
    }
}
