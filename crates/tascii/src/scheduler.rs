//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

/// this file is used to track running task states
///
use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::Duration,
};

use dal::ID;

use crossbeam_channel::{Receiver, Sender};

use tracing::{debug, error, info, trace, warn};

use crate::{
    executors,
    runtime::Runtime,
    task_runtime::{RuntimeTask, TaskState},
    workflows::TaskError,
};

#[allow(dead_code)]
pub enum TaskMessage {
    /// Notification that work was completed for <task_id>
    Complete(ID),

    /// Instruction to cancel <task_id> if there are any active contracts for it
    Cancel(ID),

    /// <task_id, contract_id>
    /// States that the given contract was not fulfilled
    /// because of an early failure
    Failure(ID, TaskError),

    /// <task_id, contract_id>
    /// States that the given contract may have timed out,
    /// and should be revoked if it is not marked as complete
    Timeout(ID),

    /// <task_id>
    /// Says to revoke the contract, provided it is not already complete
    ///
    /// Puts the task into the `Failed` metastate
    Stop(ID, TaskError),

    /// Heartbeat message, comes with implicit expectation
    /// that another Heartbeat will be scheduled
    Heartbeat(),

    /// <task_id>
    /// The task by the given ID is wanted complete,
    /// so the dependency graph of this task should
    /// be started if not yet started, or
    /// a message stating why the target can't be
    /// meaningfully progressed on should be sent to
    /// the admin
    Target(ID),

    Diagnostic(&'static str),

    /// Says .1 should depend on .0 in the task graph
    Depend(ID, ID), // TODO: use this

    /// Says .1 should NOT depend on .0 in the task graph
    UnDepend(ID, ID),
}

/// Tracks all in-flight tasks and their statuses
pub struct Orchestrator {
    running_tasks: HashSet<ID>,

    // that clears itself as tasks execute,
    // gives effectively a cached back-ref
    //
    // if a dependent is added, this should be
    // updated to reflect it in order to have
    // a chance of being picked up by an already-
    // active target
    tx: Sender<TaskMessage>,
    rx: Receiver<TaskMessage>,

    pub runtime_ref: Option<std::pin::Pin<&'static Runtime>>,
}

impl Orchestrator {
    pub(crate) fn new(tx: Sender<TaskMessage>, rx: Receiver<TaskMessage>) -> Self {
        Self {
            running_tasks: Default::default(),
            tx,
            rx,
            runtime_ref: None,
        }
    }

    /// Contract: the yielded pin must not be used after the
    /// holding Runtime is deallocated
    fn runtime(&self) -> &'static Runtime {
        self.runtime_ref.unwrap().get_ref()
    }

    /// Runs for lifetime of program, handles scheduling
    /// and handing out contracts
    ///
    /// This function should be very careful to not panic,
    /// as even though the system is resilient to
    /// flaps, it would cause a rebuild if this breaks
    pub fn task_loop(&mut self) {
        self.schedule_heartbeat(Duration::from_millis(500));

        while let Ok(msg) = self.rx.recv() {
            match msg {
                TaskMessage::Heartbeat() => {
                    trace!("Orchestrator got a heartbeat");
                    // schedule the next heartbeat to a sensical amount of time in the future
                    self.schedule_heartbeat(Duration::from_millis(1000));

                    // we take this as a chance to poll our targets and renew them

                    for t in self.runtime().get_targets() {
                        let _ = self.tx.send(TaskMessage::Target(t));
                    }
                }

                TaskMessage::Complete(task_id) => {
                    info!(
                        "Orchestrator heard that task {} was completed, so committing it",
                        task_id
                    );

                    let res = self.runtime().with_task(task_id, |t| {
                        //t.status() = TaskState::Done;
                        t.commit()
                    });

                    if let Err(e) = res {
                        tracing::error!(
                            "Couldn't set/commit task state for {}, error was {}",
                            task_id,
                            e.to_string()
                        );
                    }

                    // schedule any dependents of the task that are ready to run
                    let depends_for = self
                        .runtime()
                        .with_task(task_id, |t| t.waiting_for.clone())
                        .expect("failed to find task?");

                    for task in depends_for {
                        if self.running_tasks.contains(&task) {
                            // do nothing, task is already running
                        } else {
                            // we should check if the task should be started
                            let should_start = self
                                .runtime()
                                .with_task_mut(task, |t| {
                                    t.waiting_for.remove(&task_id);

                                    if t.waiting_for.is_empty() {
                                        debug!("Checking if t is complete...");
                                        let c = !t.is_complete(); // the task has not yet been
                                                                  // run to completion, and we
                                                                  // aren't currently running it,
                                                                  // so we should start it
                                        debug!("Returned from is_complete(), value {c}");
                                        c
                                    } else {
                                        false
                                    }
                                })
                                .expect("failed to find task?");

                            if should_start {
                                self.run_task(task);
                            }
                        }
                    }

                    self.runtime().unset_target(task_id);
                }

                TaskMessage::Cancel(task_id) => {
                    // user asked to cancel the task, so we find the matching contract and issue a
                    // revocation
                    info!("Orchestrator was asked to cancel task {}", task_id);

                    let _ = self.tx.send(TaskMessage::Stop(
                        task_id,
                        TaskError::Reason("user canceled the task".to_owned()),
                    ));
                }

                TaskMessage::Timeout(task_id) => {
                    debug!(
                        "Orchestrator processed a timeout timer for task {}",
                        task_id
                    );

                    let _ = self
                        .tx
                        .send(TaskMessage::Stop(task_id, TaskError::Timeout()));
                }

                TaskMessage::Failure(task_id, reason) => {
                    info!(
                        "Orchestrator processed a failure notification for task {}, reason {:?}",
                        task_id, reason
                    );

                    let _ = self.tx.send(TaskMessage::Stop(task_id, reason));
                }

                TaskMessage::Stop(task_id, reason) => {
                    let stopped = self
                        .runtime()
                        .with_task(task_id, |t| t.cancel(reason.clone()))
                        .expect("couldn't find task?");

                    let summary = self
                        .runtime()
                        .with_task(task_id, |t| t.proto.task_ref().summarize(task_id))
                        .unwrap_or("couldn't fetch".to_owned());

                    if let Ok(_e) = stopped {
                        warn!("STOPPED (canceled) task {summary}, with the reason {reason:?}. Intervention may be required");
                    }
                }

                TaskMessage::Target(task_id) => {
                    self.target(task_id);
                }
                _ => {
                    warn!("Bad message sent to multitasker");
                }
            }
        }

        debug!("task loop loops")
    }

    fn schedule_heartbeat(&self, when: Duration) {
        let tx = self.tx.clone();
        trace!("Orchestrator schedules another heartbeat");

        // this can be in tokio since it's cheaper to do this than to have a thread-per
        executors::spawn_on_tascii_tokio_primary(async move {
            tokio::time::sleep(when).await;
            tx.send(TaskMessage::Heartbeat())
                .expect("Couldn't send a heartbeat message to self");
        }); // let it run
    }

    /// Tries to start scheduling on the path to completing this target
    fn target(&mut self, target_task_id: ID) {
        debug!("Targetting task {target_task_id}");

        let mut schedule_set = HashSet::<ID>::new();

        let mut required_tasks = VecDeque::from([target_task_id]);

        let mut depset: HashMap<ID, HashSet<ID>> = HashMap::new();

        'task_loop: while let Some(task_id) = required_tasks.pop_front() {
            let task = self.runtime().get_task(task_id);

            let task = if let Ok(task) = task {
                task
            } else {
                error!("A task by id {task_id} was referred to but did not exist in database");
                continue 'task_loop;
            };

            if schedule_set.contains(&task_id) {
                // this task is already going to be scheduled this round,
                // so we don't need to check it again
                continue 'task_loop;
            }

            let mut r = task.get_mut().expect("couldn't open task");

            if r.status() != TaskState::Ready {
                // we can't run this task anyway
                //
                // this is our "recursive base case" so we don't traverse the entire tree,
                // and limit ourselves to the incomplete edge/leaves
                continue 'task_loop;
            }

            r.waiting_for.clear();

            // add the things it depends on back into the queue, try to move them toward done
            // if there are no tasks in that set that are in the "Ready" state, then this task
            // is stalled
            for dependency in r.depends_on.clone() {
                let was_new = depset.entry(dependency).or_default().insert(task_id);

                let status = self
                    .runtime()
                    .with_task(dependency, |t| t.status())
                    .expect("couldn't find task?");

                if status == TaskState::Done {
                    // this task is satisfied if everything is like this
                } else if status == TaskState::Failed {
                    // a task we depend on is in the failed state, so we are unsatisfiable
                    warn!("Can't finish target {target_task_id}, since task {dependency} (dependency of {task_id}) is in the failed state");

                    r.waiting_for.insert(dependency);
                } else if status == TaskState::Ready {
                    // we can run, but we need this other one to finish first
                    r.waiting_for.insert(dependency);
                }

                if was_new {
                    // this isn't a cycle, but we need to check if we need to still run this or
                    // if it was already included. We also need to check if we should queue this
                    // task
                } else {
                    error!("Task DAG was not actually an AG, loop was detected between task {task_id} and task {dependency}");
                    return;
                }
            }

            if r.waiting_for.is_empty() {
                // we can schedule the task, since it has no unfinished dependencies
                schedule_set.insert(task_id);
            }

            // not strictly necessary, but persisting waiting_for here makes debugging nicer if
            // things fall over
            //
            // this is ultimately not truly necessary, since when standing back up waiting_for is
            // rebuilt anyway by calling target() on all logically pending targets
            //r.commit().expect("TODO: handle task commit fail here");
        }

        for tid in schedule_set {
            if !self.running_tasks.contains(&tid) {
                tracing::trace!("starts run of task {tid}");
                self.run_task(tid);
            } else {
                tracing::debug!("was already running task {tid}");
            }
        }
    }

    /// Unconditionally starts the referenced tasks, whether or not dependencies are cleared
    /// Consider dependency clearance a precondition
    fn run_all<Iterable: IntoIterator<Item = ID>>(&mut self, tasks: Iterable) {
        for task_id in tasks {
            // check if a contract already exists
            self.run_task(task_id);
        }
    }

    /// Unconditionally runs the referenced task, whether or not dependencies are cleared
    /// Consider dependency clearance a precondition
    fn run_task(&mut self, task_id: ID) {
        let mut shouldnt_run = self.running_tasks.contains(&task_id);
        shouldnt_run = shouldnt_run
            || self
                .runtime()
                .with_task(task_id, |t| t.status() != TaskState::Ready)
                .expect("couldn't find task?");

        if shouldnt_run {
            warn!("Not running task {task_id} because either it is currently running or is in a prior run state");
            return;
        }

        info!("Starts run of task {task_id}");

        let rt = self.runtime(); // rearrange here to avoid reborrowing self and making bchecker
                                 // unhappy
        self.running_tasks.insert(task_id);

        let _thread = std::thread::spawn(move || RuntimeTask::run(task_id, rt));
    }

    /// Get the taskstate of the given task ID
    fn state_of(&self, task_id: ID) -> TaskState {
        self.runtime()
            .with_task(task_id, |t| t.status())
            .expect("couldn't find task?")
    }

    fn states_of(&self, ids: &HashSet<ID>) -> HashMap<ID, TaskState> {
        let mut states = HashMap::new();

        for id in ids.iter() {
            states.insert(*id, self.state_of(*id));
        }

        states
    }
}
