use dal::ID;
use std::{env::args, time::Duration};

use common::prelude::tracing;
use tascii::prelude::*;
use tracing::debug;

tascii::mark_task!(Waiter);
#[derive(Clone, Debug, Hash, Serialize, Deserialize)]
struct Waiter {
    dir: String,
}

impl Runnable for Waiter {
    type Output = String;

    fn run(&mut self, context: &Context) -> Result<Self::Output, TaskError> {
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

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("waiter task").versioned(1)
    }

    fn summarize(&self, _id: ID) -> String {
        "dfsjl".to_string()
    }
}

tascii::mark_task!(Printer);
#[derive(Clone, Debug, Hash, Serialize, Deserialize)]
struct Printer {
    print: String,
}

impl Runnable for Printer {
    type Output = String;

    fn run(&mut self, _context: &Context) -> Result<Self::Output, TaskError> {
        debug!("printer starts");

        std::thread::sleep(Duration::from_secs_f64(0.5));

        tracing::info!("printer has {}", self.print);

        std::thread::sleep(Duration::from_secs_f64(0.5));

        Ok(format!("printer had value {} given to it", self.print))
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("printer task").versioned(1)
    }

    fn summarize(&self, _id: ID) -> String {
        "jfalsdfk".to_string()
    }
}

pub fn run_tests(rt: &'static Runtime) {
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
    let _ = res
        .unwrap()
        .to_typed::<Result<String, TaskError>>()
        .unwrap()
        .wait()
        .unwrap();
}
