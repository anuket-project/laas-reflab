// Copyright (c) 2023 University of New Hampshire
// SPDX-License-Identifier: MIT
#![allow(dead_code, clippy::await_holding_lock, async_fn_in_trait)]
#![feature(
    async_iterator,
    result_flattening,
    ptr_metadata,
    unboxed_closures,
    panic_backtrace_config,
    update_panic_count,
    panic_can_unwind,
    let_chains,
    generic_arg_infer
)]

pub mod executors;
pub mod task_trait;

mod oneshot;
mod runtime;
mod scheduler;
mod workflows;

mod task_runtime;
mod task_shim;

#[allow(deprecated)]
use std::{
    cell::RefCell,
    panic::{BacktraceStyle, PanicInfo},
};

pub mod prelude {
    pub use crate::task_trait::{AsyncRunnable, Runnable, TaskIdentifier};

    pub use crate::task_shim::RunnableHandle;
    pub use crate::workflows::{Context, TaskError};

    pub use crate::runtime::Runtime;

    pub use serde::{Deserialize, Serialize};

    pub use uuid::Uuid;
}

use parking_lot::RwLock;

#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

extern crate lazy_static;

use crate::runtime::Runtime;

thread_local! {
    static CAPTURED_CONTEXT: RefCell<bool> = const { RefCell::new(true) };
}

// TODO: Update to [`PanicHookInfo`] and remove `#[allow(deprecated)]`
#[allow(deprecated)]
fn panic_hook(info: &PanicInfo<'_>) {
    LOCAL_HOOK.with(|m| {
        let g = m.lock();

        if let Some(lh) = g.as_ref() {
            lh(info)
        } else if let false = CAPTURED_CONTEXT.with_borrow(|b| *b)
            && let Some(hook) = &*DEFAULT_HOOK.read()
        {
            hook(info)
        } else {
            let style = if !info.can_unwind() {
                Some(BacktraceStyle::Full)
            } else {
                std::panic::get_backtrace_style()
            };

            // The current implementation always returns `Some`.
            let location = info.location();
            if let Some(location) = location {
                let msg = match info.payload().downcast_ref::<&'static str>() {
                    Some(s) => *s,
                    None => match info.payload().downcast_ref::<String>() {
                        Some(s) => &s[..],
                        None => "Box<dyn Any>",
                    },
                };
                let thread = std::thread::current();
                let name = thread.name().unwrap_or("<unnamed>");

                let mut output = String::new();

                use std::fmt::Write;

                let _ = writeln!(
                    &mut output,
                    "thread '{name}' panicked at '{msg}', {location}"
                );

                match style {
                    Some(BacktraceStyle::Short)
                    | Some(BacktraceStyle::Full)
                    | Some(BacktraceStyle::Off) => {
                        let bt = std::backtrace::Backtrace::force_capture().to_string();

                        let _ = writeln!(output, "{bt}");
                    }
                    Some(_) => {}
                    None => {}
                }

                tracing::error!("Panic within runtime:\n{output}");
            }
        }
    });
}

#[allow(deprecated)]
type BoxedPanic = Box<dyn Fn(&PanicInfo<'_>) + Sync + Send + 'static>;

static DEFAULT_HOOK: RwLock<Option<BoxedPanic>> = RwLock::new(None);

thread_local! {
    static LOCAL_HOOK: parking_lot::Mutex<Option<BoxedPanic>> = parking_lot::Mutex::new(None);
}

#[allow(deprecated)]
pub fn set_local_hook<F>(f: Option<Box<F>>)
where
    F: Fn(&PanicInfo<'_>) + Sync + Send + 'static,
{
    LOCAL_HOOK.with(|v| {
        let mut g = v.lock();
        if let Some(v) = f {
            let v: Box<dyn Fn(&PanicInfo<'_>) + Sync + Send + 'static> = Box::new(*v);
            *g = Some(v);
        } else {
            *g = None;
        }
    })
}

pub fn init(name: &'static str) -> &'static Runtime {
    let rt = Runtime::new(name);

    *DEFAULT_HOOK.write() = Some(std::panic::take_hook());

    std::panic::set_hook(Box::new(panic_hook));

    std::thread::spawn(|| {
        rt.start_task_loop();
    });

    rt
}
