use common::prelude::tracing;
use dal::{new_client, AsEasyTransaction, FKey};
use maplit::hashmap;
use models::inventory::Host;
use pyo3::{
    pyclass, pymethods,
    types::{IntoPyDict, PyModule},
    IntoPy, Python,
};
use tascii::{prelude::*, task_trait::AsyncRunnable};

use crate::resource_management::{cobbler::CobblerConfig, mailbox::Endpoint};

#[derive(Debug, Hash, Clone, Serialize, Deserialize)]
pub struct CobblerSetConfiguration {
    pub host_id: FKey<Host>,
    pub config: CobblerConfig,
    pub endpoint: Endpoint,
}

#[pyclass]
struct StdoutToTracing;

#[pymethods]
impl StdoutToTracing {
    fn write(&self, data: &str) {
        tracing::info!("Message from python: {data}")
    }
}

tascii::mark_task!(CobblerSetConfiguration);
impl AsyncRunnable for CobblerSetConfiguration {
    type Output = ();

    async fn run(&mut self, _context: &Context) -> Result<Self::Output, TaskError> {
        tracing::info!(
            "setting cobbler configuration for host {:?}, config is: {:#?}, and is available at {:?}",
            self.host_id,
            self.config,
            self.endpoint
        );

        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        let host = self.host_id.get(&mut transaction).await.unwrap();
        let pres = Python::with_gil(move |py| {
            let pmod = PyModule::from_code(
                py,
                include_str!("../utils/cobbler.py"),
                "cobbler.py",
                "cobbler",
            );
            let _pmod = pmod.expect("couldn't import cobbler module");
            let sys = py.import("sys").expect("couldn't import sys");
            sys.setattr("stdout", StdoutToTracing.into_py(py))
                .expect("couldn't set stdout");

            let host_name = host.server_name.clone();

            let config::CobblerConfig {
                address,
                url,
                username,
                password,
                api_username,
                api_password,
            } = config::settings().cobbler.clone();

            let locals = hashmap! {
                "config" => hashmap! {
                    "url" => url.clone(),
                    "user" => api_username,
                    "password" => api_password,
                }.into_py(py),
                "cobbler_profile" => self.config.image.clone().into_py(py),
                "cobbler_kargs" => self.config.kernel_args.clone().into_py(py),
                "host_name" => host_name.into_py(py),
            }
            .into_py_dict(py);
            let expr = r#"
import os
cwd = os.getcwd()
print(cwd)

from cobbler import CobblerAction

ca = CobblerAction(config)
if ca.profile_exists(cobbler_profile):
    ca.set_system_profile(host_name, cobbler_profile)
    ca.set_system_args(host_name, cobbler_kargs)
    ca.set_netboot(host_name)
    
else:
    raise Exception("profile did not exist, the provided profile was named " + cobbler_profile)
            "#;

            tracing::info!("gives that the cobbler server URL is {}", url);
            tracing::info!("sets the cobbler image to {}", self.config.image);

            let pyres = py.run(expr, None, Some(locals));

            match pyres {
                Ok(()) => {
                    tracing::info!("Set cobbler config!");
                    Ok(())
                }
                Err(e) => {
                    let tb = e.traceback(py);
                    tracing::error!("failed to run cobbler action, error was: {e:?}");
                    if let Some(tb) = tb {
                        tracing::error!("traceback: {}", tb.format().unwrap());
                    };
                    Err(TaskError::Reason(format!("pyerr: {e:?}")))
                }
            }
        });

        transaction.commit().await.unwrap();

        pres
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("CobblerSetConfigurationTask").versioned(1)
    }
}
