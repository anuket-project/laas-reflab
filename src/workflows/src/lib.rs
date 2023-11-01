//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

#![feature(result_flattening, iter_intersperse, if_let_guard, async_fn_in_trait)]

//#![allow(dead_code, unused_variables, unused_imports, unused_mut)]

pub mod cleanup_booking;
pub mod deploy_booking;
pub mod entry;
pub mod resource_management;
pub mod test_tascii;
pub mod utils;

use tascii::{prelude::*, task_trait::AsyncRunnable};

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}

pub fn retry_for<R: AsyncRunnable + 'static>(
    r: R,
    c: &Context,
    times: usize,
    wait_secs: u64,
) -> Result<R::Output, TaskError>
where
    R::Output: Sync + Send + std::panic::RefUnwindSafe,
{
    let mut last_error = TaskError::Reason("task never ran, retried 0 times?".to_owned());

    for _ in 0..times {
        let r = c.spawn(r.clone()).join();

        match r {
            Ok(v) => return Ok(v),
            Err(e) => {
                last_error = e;
                std::thread::sleep(std::time::Duration::from_secs(wait_secs));
                continue;
            }
        }
    }

    Err(last_error)
}
