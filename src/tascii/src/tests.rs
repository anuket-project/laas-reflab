//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use std::{env::args, time::Duration};

use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::{
    init,
    task_trait::{self, Runnable, TaskIdentifier, TaskRegistered},
    prelude::TaskError,
};

//#[test]
pub async fn workflows_1() {
    let rt = init("primary");

    let id = rt.enroll(
        Waiter {
            dir: "default".into(),
        }
        .into(),
    );

    let args: Vec<String> = args().collect();

    if args
        .get(1)
        .cloned()
        .unwrap_or(String::from("nocreate"))
        .as_str()
        == "create"
    {
        rt.set_target(id);
    }

    let res = rt.with_task(id, |t| t.result.clone());

    debug!("test waits on end of primary");
    let _ = res.unwrap().to_typed::<Result<String, TaskError>>().unwrap().wait().unwrap();

    //assert!(r == "hello");
}

inventory::submit! { crate::task_trait::register_task::<Waiter>() }
unsafe impl TaskRegistered for Waiter {}
#[derive(Clone, Debug, Hash, Serialize, Deserialize)]
struct Waiter {
    dir: String,
}

impl Runnable for Waiter {
    type Output = String;

    fn summarize(&self, id: dal::ID) -> String {
        format!("no")
    }

    fn run(
        &mut self,
        context: &crate::workflows::Context,
    ) -> Result<Self::Output, crate::workflows::TaskError> {
        debug!("started run for waiter");
        //context.spawn(t);
        for i in 0..1000 {
            let jhs: Vec<_> = (0..300)
                .map(|j| {
                    let p = Printer {
                        print: format!("given outer {i} inner {j}"),
                    };
                    //let p = Printer { print: String::from("same")};

                    debug!(
                        "going to spawn a printer within context, printer is {:?}",
                        p
                    );
                    let jh = context.spawn(p);

                    debug!("about to join it");

                    //std::thread::sleep(Duration::from_secs_f64(0.5));

                    jh
                })
                .collect();

            for jh in jhs {
                let val = jh.join()?;

                tracing::info!("finished join task, printer returned to us {val}");
            }

            tracing::info!("did {} tasks", i * 500);
        }

        debug!("finished run for waiter");

        Ok("hello".into())
    }

    fn identifier() -> crate::task_trait::TaskIdentifier {
        TaskIdentifier::named("waiter task").versioned(1)
    }
}

/*lazy_static! {
    tasktrait::register(TaskIdentifier::named("waiter task").versioned(1), |st| Waiter::deserialize(st.data));
}*/

inventory::submit! { crate::task_trait::register_task::<Printer>() }
unsafe impl TaskRegistered for Printer {}
#[derive(Clone, Debug, Hash, Serialize, Deserialize)]
struct Printer {
    print: String,
}

impl Runnable for Printer {
    type Output = String;

    fn summarize(&self, id: dal::ID) -> String {
        format!("no")
    }

    fn run(
        &mut self,
        _context: &crate::workflows::Context,
    ) -> Result<Self::Output, crate::workflows::TaskError> {
        debug!("printer starts");

        std::thread::sleep(Duration::from_secs_f64(0.5));

        tracing::info!("printer has {}", self.print);

        std::thread::sleep(Duration::from_secs_f64(0.5));

        Ok(format!("printer had value {} given to it", self.print))
    }

    fn identifier() -> crate::task_trait::TaskIdentifier {
        TaskIdentifier::named("printer task").versioned(1)
    }
}
