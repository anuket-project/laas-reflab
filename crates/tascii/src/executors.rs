//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

/// TASCII is an async library in sync clothing, providing an async interface for tasks that
/// just want to "do" async without worrying about doing blocking operations
///
/// Naturally, this presents some difficulties. We built TASCII to use Tokio,
/// but you can't do `async -> sync -> async` sandwiches nicely in Tokio,
/// or in fact in Rust in general.
///
/// We also want to have many of the runtime support functions
/// (particularly for Context!) be available in both sync and async contexts
///
/// Thus, we do something a bit...unconventional.
///
/// Every task, even async ones, is run within a thread within a Rayon pool.
/// That thread's work unit contains a current_thread tokio runtime, which itself allows
/// the task to do async "things".
///
/// If that thread calls a sync API within TASCII, that sync api will send an async task into
/// TASCII's primary Tokio runtime, with that task being given a (sync) channel to send a
/// completion notification through.
///
/// The sync end of the API then waits for that message from the channel, blocking
///
/// This means that tasks themselves, which are "async" can do blocking operations--
/// since they are already isolated to their own thread, and can't poison any
/// "shared" executor with their blocking. This allows users to largely ignore
/// async "color" when writing tasks, and just "do the thing"
///
/// This module tries to wrap up the various abstractions for bridging these two
/// warring contexts into a nice, tidy package
use std::{panic::AssertUnwindSafe, process::exit, sync::atomic::compiler_fence};

use dashmap::DashMap;
use futures_util::{Future, FutureExt};

use tracing::{
    error,
    log::{trace, warn},
};

pub fn spawn_on_tascii_tokio<T, F>(name: &'static str, f: F) -> T
where
    T: Send + Sync + 'static,
    F: Future<Output = T> + Send + 'static,
{
    spawn_on_tascii_tokio_options(name, f, RtOptions::default())
}

pub fn spawn_on_tascii_tokio_options<T, F, N>(name: N, f: F, options: RtOptions) -> T
where
    T: Send + Sync + 'static,
    F: Future<Output = T> + Send + 'static,
    N: Into<String>,
{
    let trt = get_tokio_runtime(name.into(), options);

    let (tx, rx) = crossbeam_channel::bounded(1);

    trace!("Made channel");

    trt.spawn(async move {
            trace!("Running future");

            let ret = AssertUnwindSafe(f).catch_unwind().await;

            trace!("Ran future");
            // at this point, the future itself should be completely consumed--even if it panicked

            compiler_fence(std::sync::atomic::Ordering::SeqCst); // just don't want to leave this
                                                                 // base uncovered

            if ret.is_err() {
                exit(-1);
            }

            match ret {
                Ok(v) => {
                    // we assert unwind safety here since the immediate action, if T is given back
                    // to us, is to forget it--no destructors get called
                    let _send_res = std::panic::catch_unwind(AssertUnwindSafe(move || {
                        let send_res = tx.send(v);
                        if send_res.is_err() {
                            warn!("Failed to send into channel");
                        }
                    }));
                }

                Err(e) => {
                    error!("Got a horrible error within tascii, a panic occurred within a runtime function running
                        within the primary tokio runtime. The best-effort formatting of the error is this: {e:?}");
                    // we can do our best to try to format the error, so do that fallibly here
                    std::mem::forget(std::panic::catch_unwind(AssertUnwindSafe(|| {
                    })));

                    // can't risk dropping the error, so forget it
                    std::mem::forget(e);

                    // something has gone horribly wrong
                    // but we also don't want to kill the entire program,
                    // so instead we drop our side of the channel and hope the other side
                    // gets the memo (it will, it's like 5 lines below us)
                    std::mem::forget(std::panic::catch_unwind(move || {
                        std::mem::drop(tx);
                    }));
                }
            }
        });

    trace!("Spawned the future onto the runtime");

    rx.recv().expect("a channel to get back the result of a tokio task unexpectedly was closed, the thing inside must have panicked")
}

/// Be VERY careful that the async
/// closure you pass can not panic, or
/// that it does not borrow any values going in
/// If it could possibly panic, then the
pub fn spawn_on_tascii_tokio_primary<T, F>(f: F) -> T
where
    T: Send + Sync + 'static,
    F: Future<Output = T> + Send + 'static,
{
    spawn_on_tascii_tokio("primary", f)
}

lazy_static::lazy_static! {
    static ref RUNTIMES: DashMap<String, &'static tokio::runtime::Runtime> = DashMap::new();
}

pub struct RtOptions {
    pub threads: Option<usize>,
}

impl std::default::Default for RtOptions {
    fn default() -> Self {
        Self { threads: Some(8) }
    }
}

pub fn make_tokio_mt_runtime(options: RtOptions) -> &'static tokio::runtime::Runtime {
    Box::leak(Box::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(options.threads.unwrap_or(8))
            .build()
            .expect("TASCII couldn't start a Tokio runtime"),
    ))
}

pub fn get_tokio_runtime(context: String, options: RtOptions) -> &'static tokio::runtime::Runtime {
    RUNTIMES
        .entry(context)
        .or_insert_with(|| make_tokio_mt_runtime(options))
        .value()
}

type BoundLess<'big, 'small> = [&'small &'big (); 0];
