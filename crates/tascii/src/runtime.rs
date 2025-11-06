//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use std::{
    collections::{HashMap, HashSet},
    fs::read_to_string,
    sync::Arc,
    time::Duration,
};

use crossbeam_channel::Sender;
use dal::{AsEasyTransaction, DBTable, ID};
use dashmap::DashSet;
use futures_util::future::BoxFuture;
use parking_lot::{Mutex, RwLock, RwLockWriteGuard};
use tracing::{debug, warn};
use write_to_file::WriteToFile;

use crate::{
    executors,
    scheduler::{self, Orchestrator, TaskMessage},
    task_runtime::{RuntimeTask, TaskGuard, TaskGuardInner, TaskState},
    task_shim::RunnableHandle,
};

pub struct Runtime {
    orchestrator: Mutex<Orchestrator>,

    tx: Sender<TaskMessage>,

    targets: DashSet<ID>,

    all_tasks: Mutex<HashMap<ID, Arc<TaskGuardInner>>>,
    all_task_ids: Mutex<HashSet<ID>>,

    identity: &'static str,
}

impl Runtime {
    pub fn enroll(&'static self, v: RunnableHandle) -> ID {
        debug!("enrolls task");
        // enroll the task within the given runtime
        self.create(v)
    }

    /// Says a must happen before b can start
    pub fn depend(&'static self, a: ID, b: ID) {
        let res = self.tx.send(TaskMessage::Depend(a, b));

        if let Err(e) = res {
            tracing::error!(
                "Couldn't send a depends message for {a} and {b}, error was {e:?} as channel closed"
            );
        }
    }

    pub fn new(runtime_identity: &'static str) -> &'static Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let inner = Orchestrator::new(tx.clone(), rx);

        let s = Self {
            identity: runtime_identity,
            orchestrator: Mutex::new(inner),
            tx,
            targets: DashSet::new(),
            all_tasks: Mutex::new(HashMap::new()),
            all_task_ids: Mutex::new(HashSet::new()),
        };

        let r = Box::leak(Box::new(s));
        let r = &*r;

        let pinned = std::pin::Pin::new(r);

        r.orchestrator.lock().runtime_ref = Some(pinned);

        r.recover();

        r
    }

    /// Start executing the graph starting at (id)
    /// This will not start any tasks who may be blocking the descendent tasks
    /// but who are not descendents of (id)
    #[allow(dead_code)]
    pub fn runfrom(_id: ID) {}

    /// When we want a "target" reached,
    /// we hand its task ID to this function
    ///
    /// This function then searches for the
    /// undone slice that can begin executing that are
    /// a predep for the current function
    pub fn set_target(&'static self, id: ID) {
        let res = self.with_task(id, |t| t.commit());

        if let Err(e) = res {
            tracing::error!("Error setting target to {id}, error was {}", e.to_string());
        }

        self.targets.insert(id);

        self.save_targets();

        let _ = self.tx.send(scheduler::TaskMessage::Target(id));
    }

    /// Send a message directly to the scheduler for this runtime
    /// on this node
    ///
    /// WARN: this should be considered an internal fn to the rt!
    pub(crate) fn send(
        &'static self,
        msg: TaskMessage,
    ) -> Result<(), crossbeam_channel::SendError<TaskMessage>> {
        self.tx.send(msg)
    }

    pub fn unset_target(&self, id: ID) {
        self.targets.remove(&id);

        self.save_targets();
    }

    #[allow(dead_code)]
    pub fn modify_state(_id: ID, _newstate: TaskState) {}

    /// Gets a deduplicated set of taskguards for the list of IDs
    pub async fn get_all(&'static self, all: &[ID]) -> Vec<TaskGuard> {
        let _as_set: HashSet<ID> = all.iter().copied().collect();

        let mut r = Vec::new();

        for id in all.iter() {
            if let Ok(tg) = self.get_task(*id) {
                r.push(tg);
            }
        }

        r
    }

    /// assumes that `all` has been deduplicated (no duplicate task guards)
    pub fn lock_all_mut<'b>(
        &'static self,
        all: &'b [TaskGuard],
    ) -> HashMap<ID, RwLockWriteGuard<'b, RuntimeTask>> {
        let mut mapped = HashMap::new();

        for elem in all {
            let locked = elem.get_mut().expect("poisoned lock?");
            mapped.insert(locked.id(), locked);
        }

        mapped
    }

    pub fn create(&'static self, inner: RunnableHandle) -> ID {
        debug!("creates task");
        let task = self
            .new_task(inner, self)
            .expect("The task could not be created in the database");

        task.context.set_runtime(self);

        let id = task.id();
        debug!("saving the task");

        let g = self.save_task(task);
        g.get_mut()
            .expect("poisoned lock?")
            .commit()
            .expect("couldn't create a task");

        debug!("Create task, now has id {id}");

        id
    }

    pub fn recover(&self) {
        // re-set the targets we had last time
        for targ in self.load_targets() {
            self.targets.insert(targ);

            // TODO: revisit recovery mechanism
        }
    }

    #[allow(dead_code)]
    fn save_target(&self, id: ID) {
        self.targets.insert(id);

        self.save_targets();
    }

    fn save_targets(&self) {
        let as_vec: Vec<ID> = self.targets.iter().map(|ent| *ent).collect();
        let serialized = serde_json::to_string(&as_vec).unwrap();

        let identity = self.identity;
        let _ = serialized.write_to_file(format!("{identity}-targets.json"));
    }

    fn load_targets(&self) -> Vec<ID> {
        let identity = self.identity;
        let s = read_to_string(format!("{identity}-targets.json")).unwrap_or("[]".to_owned());
        let deserialized: Vec<ID> = serde_json::from_str(&s).unwrap_or(vec![]);

        deserialized
    }

    pub fn get_targets(&self) -> Vec<ID> {
        self.targets.iter().map(|e| *e).collect()
    }

    pub fn start_task_loop(&self) {
        self.orchestrator.lock().task_loop();
        tracing::debug!("Task loop broke");
    }

    pub async fn async_with_task<'env, T>(
        &'static self,
        id: ID,
        f: impl for<'p> FnOnce(&'p RuntimeTask, BoundLess<'env, 'p>) -> BoxFuture<'p, T>,
    ) -> Result<T, anyhow::Error> {
        let t = self.get_task_async(id).await?;
        let r = t.get_ref()?;

        let ref_r = &*r;

        let res = f(ref_r, []).await;

        Ok(res)
    }

    pub async fn async_with_task_mut<'env, T, F>(
        &'static self,
        id: ID,
        f: F,
    ) -> Result<T, anyhow::Error>
    where
        F: for<'p> FnOnce(&'p mut RuntimeTask, BoundLess<'env, 'p>) -> BoxFuture<'p, T> + Send,
    {
        let t = self.get_task_async(id).await?;
        let mut r = t.get_mut()?;

        let ref_r = &mut *r;

        let res = f(ref_r, []).await;

        Ok(res)
    }

    pub fn with_task_mut<T>(
        &'static self,
        id: ID,
        f: impl FnOnce(&mut RuntimeTask) -> T,
    ) -> Result<T, anyhow::Error> {
        let t = self.get_task(id)?;

        let mut r = t.get_mut()?;
        let ref_r = &mut *r;

        Ok(f(ref_r))
    }

    pub fn with_task<T>(
        &'static self,
        id: ID,
        f: impl FnOnce(&RuntimeTask) -> T,
    ) -> Result<T, anyhow::Error> {
        debug!("Getting task");
        let t = self.get_task(id)?;
        debug!("Got task, waiting for ref");

        let r = t.get_ref()?;
        debug!("Got ref");

        let ref_r = &*r;

        Ok(f(ref_r))
    }

    pub fn get_task(&'static self, id: ID) -> Result<TaskGuard, anyhow::Error> {
        executors::spawn_on_tascii_tokio_primary(self.get_task_async(id))
    }

    /// This should not panic, as it is run within the sensitive primary
    /// tokio rt for TASCII
    ///
    /// That is still handled soundly, but will lead to memory leaks so is not graceful
    pub async fn get_task_async(&'static self, id: ID) -> Result<TaskGuard, anyhow::Error> {
        let mut all = self.all_tasks.lock();

        // tasks remove themselves from the map within a guarded section in their drop,
        // but we could still observe a dead Weak if the task starts dropping while we're in this
        // locked section
        //
        // NOTE: disregard prior comment, tasks do not remove themselves and instead leave behind a
        // dead Weak
        //
        // TODO: revisit dead Weaks

        let might_exist = all.get(&id);

        if let Some(strong) = might_exist {
            Ok(TaskGuard {
                inner: strong.clone(),
            })
        } else {
            debug!("No strong found, so need to reload it");
            let task = self.load_task(id).await;

            match task {
                Ok(task) => {
                    let lock = RwLock::new(task);
                    let inner = TaskGuardInner { task: lock };

                    let strong = Arc::new(inner);

                    all.insert(id, strong.clone());

                    Ok(TaskGuard { inner: strong })
                }
                Err(e) => Err(e),
            }
        }
    }

    pub fn save_task(&self, task: RuntimeTask) -> TaskGuard {
        let id = task.id();

        debug!("Saves task {id}");

        let inner = TaskGuardInner {
            task: RwLock::new(task),
        };
        let guard = TaskGuard {
            inner: Arc::new(inner),
        };

        self.all_tasks.lock().insert(id, guard.inner.clone());

        self.all_task_ids.lock().insert(id);

        guard.commit().expect("couldn't commit task");

        guard
    }

    pub async fn load_task(&self, id: ID) -> Result<RuntimeTask, anyhow::Error> {
        //debug!("Asked for task {id}");
        self.all_task_ids.lock().insert(id);

        let mut client = dal::new_client().await?;
        let mut trans = client.easy_transaction().await?;

        let got = RuntimeTask::get(&mut trans, id).await?;

        trans.commit().await?;

        Ok(got.into_inner())
    }

    pub fn new_task(
        &self,
        proto: RunnableHandle,
        within_rt: &'static Runtime,
    ) -> Result<RuntimeTask, anyhow::Error> {
        let t = RuntimeTask::build(proto, within_rt)?;
        let id = t.id();

        debug!("makes new task {id} with empty context");

        Ok(t)
    }

    async fn print_tasks(&'static self) {
        let ids = self.all_task_ids.lock().clone();
        let mut summaries = vec![];
        for task in ids {
            summaries.push(
                self.get_task_async(task)
                    .await
                    .map(|t| t.get_ref().expect("poisoned lock?").summarize())
                    .unwrap_or("<task not in db>".into()),
            );
        }

        warn!("Current set of tasks: ");
        for summary in summaries {
            warn!("  {}", summary);
        }
    }

    pub fn start_task_report_loop(&'static self) {
        executors::spawn_on_tascii_tokio_primary(async {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                self.print_tasks().await;
            }
        })
    }
}

type BoundLess<'big, 'small> = [&'small &'big (); 0];
