//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use std::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    panic::RefUnwindSafe,
    sync::{atomic::fence, Arc, Weak},
};

use dal::{
    web::AnyWaySpecStr,
    AsEasyTransaction,
    DBTable,
    ExistingRow,
    FKey,
    JsonModel,
    Migrate,
    NewRow,
    ID,
};

use dashmap::DashMap;

#[cfg(target_os = "linux")]
use linux_futex::{Futex, Private};

use parking_lot::Mutex;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tracing::debug;

use crate::{task_trait::TaskSafe, workflows::TaskError};

/// This is a stub that does actually point
/// to a typed oneshot container, but itself
/// does not accept or understand any type parameter
///
/// We
#[derive(Clone)]
pub struct SimpleOneshotHandle {
    pub id: ID,
}

/// We impl Deserialize/Serialize for SimpleOneshotHandle
/// for a singular reason: RuntimeTask, even though when
/// it's "instantiated" holds a StrongUntypedOneshotHandle,
/// is *serialized* holding the oneshot as a SimpleOneshotHandle
///
/// This design allows a two-step deserialization where
/// first the task and oneshot ID are deserialized,
/// and then the DynRunnable is *given* the SimpleOneshotHandle
/// to (within its own context, where it knows what T is)
/// first reload the OneShot itself, and then do a widening
/// cast from a specific OneShot<Result<T, TaskError>> to
/// an Arc<dyn OneShotAny> and finally wrap it with StrongUntypedOneshotHandle
impl<'de> Deserialize<'de> for SimpleOneshotHandle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where D: serde::Deserializer<'de> {
        let id = ID::deserialize(deserializer)?;

        Ok(Self { id })
    }
}

impl Serialize for SimpleOneshotHandle {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: serde::Serializer {
        self.id.serialize(serializer)
    }
}

#[derive(Clone)]
pub struct StrongUntypedOneshotHandle {
    stored_as: ID,
    oneshot: Arc<dyn OneShotAny>,
}

impl StrongUntypedOneshotHandle {
    pub fn simplify(&self) -> SimpleOneshotHandle {
        SimpleOneshotHandle { id: self.stored_as }
    }

    pub fn to_typed<T: TaskSafe + Sized>(&self) -> Option<&OneShot<T>> {
        self.oneshot.as_any().downcast_ref()
    }
}

impl std::fmt::Debug for StrongUntypedOneshotHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "Strong handle with an inner oneshot id of {}",
            self.stored_as
        )
    }
}

pub struct WeakUntypedOneshotHandle {
    stored_as: ID,
    oneshot: Weak<dyn OneShotAny>,
}

impl WeakUntypedOneshotHandle {
    pub fn upgrade(&self) -> Option<StrongUntypedOneshotHandle> {
        if let Some(u) = self.oneshot.upgrade() {
            Some(StrongUntypedOneshotHandle {
                stored_as: self.stored_as,
                oneshot: u,
            })
        } else {
            None
        }
    }
}

pub trait OneShotAny: Send + Sync {
    fn as_any(&self) -> &dyn std::any::Any;
}

impl<T: TaskSafe + Send + Sync> OneShotAny for OneShot<Result<T, TaskError>> {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(bound(deserialize = "T: DeserializeOwned + Serialize"))]
pub struct StoredOneShot<T: TaskSafe> {
    stored_as: FKey<DatabaseObjectWrapper<Option<T>>>,
    value: Option<T>,
}

impl<T: TaskSafe> StoredOneShot<T> {
    /// Creates a row in the db that we are stored as, and returns self
    pub async fn new() -> Result<Self, anyhow::Error> {
        let dbo = DatabaseObjectWrapper::new(None).await?;

        Ok(Self {
            stored_as: dbo,
            value: None,
        })
    }

    fn assume_exists(id: FKey<DatabaseObjectWrapper<Option<T>>>, val: T) -> Self {
        Self {
            stored_as: id,
            value: Some(val),
        }
    }

    pub async fn store(self) -> Result<(), anyhow::Error> {
        let existing = DatabaseObjectWrapper::assume_exists(self.stored_as, self.value);
        debug!("Assumed it existed");

        let mut client = dal::new_client().await?;
        let mut trans = client.easy_transaction().await?;

        debug!("About to update it in the db");
        let res = existing.update(&mut trans).await?;

        debug!("About to commit");
        trans.commit().await?;

        Ok(())
    }

    pub fn into_oneshot(self) -> OneShot<T> {
        OneShot {
            stored_as: self.stored_as,
            completed: Mutex::new(self.value.is_some()),
            announce: Futex::new(if self.value.is_some() { 0 } else { 1 }),
            value: self
                .value
                .map(|v| UnsafeCell::new(MaybeUninit::new(v)))
                .unwrap_or(UnsafeCell::new(MaybeUninit::uninit())),
        }
    }
}

unsafe impl<T: TaskSafe> Send for OneShot<T> {}
unsafe impl<T: TaskSafe> Sync for OneShot<T> {}

pub struct OneShot<T: TaskSafe + Send + Sync> {
    stored_as: FKey<DatabaseObjectWrapper<Option<T>>>,

    // Carries true if this is already completed,
    // false if not yet
    completed: Mutex<bool>,

    announce: Futex<Private>,
    value: std::cell::UnsafeCell<MaybeUninit<T>>,
}

impl<T: TaskSafe> std::ops::Drop for OneShot<T> {
    fn drop(&mut self) {
        // if it's completed, we want to dealloc the value that it holds
        if self
            .announce
            .value
            .load(std::sync::atomic::Ordering::SeqCst)
            == 0
        {
            unsafe {
                let v = self.value.get().as_mut().unwrap().assume_init_read();
                std::mem::drop(v);
            }
        }
    }
}

impl<T: TaskSafe + Send + Sync + RefUnwindSafe> OneShot<T> {
    /// Returns true if this action completes the OneShot, false if it was
    /// already completed
    pub async fn complete_with(&self, v: T) -> bool {
        let mut r = false;

        // avoid doing things that could panic inside the critical section
        let val = v.clone();

        let mut g = self.completed.lock();

        // TODO: I want to revisit this once
        // we work on making TASCII multi-node and HA,
        // since even within a single-node environment
        // this "have to persist the result before using it for later tasks"
        // bit is kind of bottleneck-y, and ideally we could just enforce
        // a total store order for task persists (or at least a serializable ordering
        // that is compatible with the task graph) and get an also-valid ordering
        // that simply may re-run more work in the event of a power disruption
        if !*g {
            debug!("The oneshot wasn't already completed, so we're completing it");
            let id = self.stored_as;
            let success = StoredOneShot::assume_exists(id, val).store().await.is_ok();

            debug!("success of storing oneshot: {success}");

            if !success {
                tracing::error!("Failed to save a oneshot");
                return false;
            }

            debug!("Storing the oneshot in memory");
            unsafe { self.value.get().as_mut().unwrap().write(v) };

            fence(std::sync::atomic::Ordering::SeqCst); // could relax this maybe

            self.announce
                .value
                .store(0, std::sync::atomic::Ordering::SeqCst);

            // we can wake up waiters on the futex even before we've dropped the lock,
            // since they read without having to hold the lock and the only
            // thing the critical section conflicts with is completion-contention,
            // which is only between the current task instance and the timeout thread
            self.announce.wake(i32::MAX); //.expect("couldn't wake waiters on a futex");

            *g = true; // mark complete

            r = true; // return that we succeeded
        }

        std::mem::drop(g);

        r
    }

    pub fn wait(&self) -> Result<T, anyhow::Error> {
        debug!(
            "waiting on futex at {:?}",
            &self.announce as *const Futex<Private>
        );

        while let 1 = self
            .announce
            .value
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            debug!(
                "within futex wait, got 1, waiting on futex at addr {:?}",
                &self.announce as *const Futex<Private>
            );
            let r = self.announce.wait(1); // wait for it to be zero
                                           //
            debug!("r from futex was {r:?}");
            //std::thread::sleep_ms(1000);
        }

        debug!(
            "finished wait on futex at {:?}",
            &self.announce as *const Futex<Private>
        );

        // can now grab the inner
        self.get()
            .ok_or("wait panics because the clone for the value inside of it panicked")
            .anyway()
    }

    pub fn get(&self) -> Option<T> {
        fence(std::sync::atomic::Ordering::SeqCst);

        if self
            .announce
            .value
            .load(std::sync::atomic::Ordering::SeqCst)
            == 1
        {
            // it hasn't been signaled, so it isn't yet complete
            return None;
        } else {
            unsafe {
                let r = self
                    .value
                    .get()
                    .as_ref()
                    .expect("should never panic, guarded by fuse")
                    .assume_init_ref();
                std::panic::catch_unwind(|| r.clone()).ok()
            }
        }
    }

    /// The canonical way to get a oneshot,
    /// this creates an empty oneshot in the database and returns an
    /// active, ready-to-complete oneshot
    pub async fn new() -> Result<Self, anyhow::Error> {
        Ok(StoredOneShot::new().await?.into_oneshot())
        //StoredOneShot { value: None, id: ID::new() }.into_oneshot()
    }
}

pub struct OneShotRegistry {
    oneshots: DashMap<ID, WeakUntypedOneshotHandle>,
}

lazy_static::lazy_static! {
    static ref REGISTRY: OneShotRegistry = OneShotRegistry { oneshots: DashMap::new() };
}

impl OneShotRegistry {
    fn instance() -> &'static OneShotRegistry {
        &REGISTRY
    }

    async fn load<T: TaskSafe + Send + Sync>(s: SimpleOneshotHandle) -> Arc<dyn OneShotAny> {
        let o: DatabaseObjectWrapper<StoredOneShot<Result<T, TaskError>>> =
            DatabaseObjectWrapper::load(s.id)
                .await
                .expect("didn't exist in db?");

        let o = o.v;

        let o: OneShot<Result<T, TaskError>> = o.into_oneshot();

        Arc::new(o)
    }

    fn new_weak_handle() -> Weak<dyn OneShotAny> {
        Weak::<OneShot<Result<(), TaskError>>>::new() as Weak<dyn OneShotAny>
    }

    /// T in this context should be Runnable::Output, this creates
    /// a OneShot<Result<T, TaskError>> within the untyped oneshot handle
    pub async fn new_task_oneshot<T: TaskSafe>() -> Result<StrongUntypedOneshotHandle, anyhow::Error>
    {
        let os = OneShot::<Result<T, TaskError>>::new().await?;
        let id = os.stored_as;
        let arcd: Arc<dyn OneShotAny> = Arc::new(os);

        Ok(StrongUntypedOneshotHandle {
            stored_as: id.into_id(),
            oneshot: arcd,
        })
    }

    pub async fn get_as_any<T: TaskSafe>(id: ID) -> StrongUntypedOneshotHandle {
        let inst = Self::instance();

        let mut mr = inst.oneshots.entry(id).or_insert_with(|| {
            WeakUntypedOneshotHandle {
                stored_as: id,
                oneshot: Self::new_weak_handle(),
            }
        });

        if let Some(v) = mr.upgrade() {
            v
        } else {
            // need to load from db, (maybe) replace the entry with what we find
            // try to find one stored to DB,
            // if we fail then raise an error since
            // at this point we should already have been created
            let a = Self::load::<T>(SimpleOneshotHandle { id }).await;

            *mr = WeakUntypedOneshotHandle {
                stored_as: id,
                oneshot: Arc::downgrade(&a),
            };

            StrongUntypedOneshotHandle {
                stored_as: id,
                oneshot: a,
            }
        }
    }
}

mod futex_polyfill {
    //! a stub futex for platforms that don't provide futex
    //! currently doesn't work, only used for development and having things
    //! compile nicely
    //!
    //! implementing a functioning polyfill of futex is left as an exercise for the reader
    use std::{
        marker::PhantomData,
        sync::{atomic::AtomicI32, Condvar},
    };

    #[derive(Debug)]
    #[allow(dead_code)]
    pub struct Futex<T> {
        pub value: AtomicI32,
        cv: Condvar,
        _p: PhantomData<T>,
    }

    impl<T> Futex<T> {
        #[allow(dead_code)]
        pub fn new<I>(v: I) -> Futex<T>
        where I: Into<AtomicI32> {
            Self {
                value: v.into(),
                _p: PhantomData::default(),
                cv: Condvar::new(),
            }
        }

        #[allow(dead_code)]
        pub fn wait(&self, v: i32) -> Result<usize, Box<dyn std::error::Error>> {
            let r = loop {
                // use the condvar if we ever want to make this work properly, but
                // we'll want to somehow wrap `value` to not be directly accessible
                //
                // the other code will need to be changed to not rely on `value`,
                // since even if we added a mutex here to guard nothing contractually prevents
                // breaking that behavior
                let lv = self.value.load(std::sync::atomic::Ordering::SeqCst);
                if lv == v {
                    // we only exit once the value is no longer the expected one
                    //
                    // use a yield_now here since we don't really care about some lost
                    // performance, and even though a proper impl would
                    // use a spin lock we
                    // a) don't fully understand the scheduler behavior on the target systems
                    // for this impl (they could bias toward us if we spin, even with spin_loop()
                    // hint)
                    // b) are often going to be waiting a little while *anyway* for the target task
                    // to complete

                    std::thread::yield_now(); // yes, I know, this is wrong, but being lazy here
                                              // isn't the end of the world
                } else {
                    break Ok(lv as usize);
                }
            };

            tracing::debug!("waiting on a futex returns!");

            r
        }

        #[allow(dead_code)]
        pub fn wake(&self, _val: i32) -> usize {
            // just do nothing for now, I know this
            // isn't actually how Futex behaves but I'd need to construct
            // a wait list and for our uses simply a regular barrier is sufficient
            //
            // we already wake by setting a different value for the futex
            //
            // this isn't quite how the regular futex behaves, but for the purposes of completable
            // we don't actually note how many we woke
            0
        }
    }

    #[derive(Debug)]
    pub struct Private {} // nice types
}

#[cfg(not(target_os = "linux"))]
use futex_polyfill::{Futex, Private};

inventory::submit! { Migrate::new(DatabaseObjectWrapper::<()>::migrations) }
#[derive(Serialize, Deserialize, Debug)]
#[serde(bound(deserialize = "T: DeserializeOwned + Serialize"))]
pub struct DatabaseObjectWrapper<T: TaskSafe> {
    id: FKey<DatabaseObjectWrapper<T>>,
    v: T,
}

impl<T: TaskSafe> JsonModel for DatabaseObjectWrapper<T> {
    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn table_name() -> &'static str {
        "tascii_database_objects"
    }
}

#[allow(dead_code)]
impl<T: TaskSafe> DatabaseObjectWrapper<T> {
    pub fn open(self) -> T {
        self.v
    }

    pub fn assume_exists(fk: FKey<Self>, value: T) -> ExistingRow<Self> {
        ExistingRow::from_existing(Self { id: fk, v: value })
    }

    pub async fn new(v: T) -> Result<FKey<Self>, anyhow::Error> {
        //let copy = v.clone();
        let id = FKey::new_id_dangling();
        //let id_copy = id;
        let mut client = dal::new_client().await?;
        let mut trans = client.easy_transaction().await?;

        let r = NewRow::new(Self { id, v }).insert(&mut trans).await?;

        trans.commit().await.map(|_| r)
    }

    pub async fn store(self) -> Result<(), anyhow::Error> {
        let mut client = dal::new_client().await?;
        let mut trans = client.easy_transaction().await?;

        let tn = <DatabaseObjectWrapper<T> as JsonModel>::table_name();
        let id = self.id;

        let fk = self.id;
        ExistingRow::from_existing(self).update(&mut trans).await?;
        trans.commit().await?;

        tracing::debug!("committed entry to db within DOW");

        Ok(())
    }

    pub async fn load(id: ID) -> Result<Self, anyhow::Error> {
        let fk: FKey<DatabaseObjectWrapper<T>> = FKey::from_id(id);

        let mut client = dal::new_client().await?;
        let mut trans = client.easy_transaction().await?;

        let r = fk.get(&mut trans).await?.into_inner();

        trans.commit().await?;

        Ok(r)
    }
}
