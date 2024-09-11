//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use std::collections::HashMap;

use models::{
    dal::{new_client, AsEasyTransaction, DBTable, FKey, ID},
    dashboard,
    inventory,
};
use pyo3::{prelude::*, types::PyAny};
use serde::{Deserialize, Serialize};
use serde_json;

use crate::{resource_management::mailbox::Endpoint, utils::python::*};
use common::prelude::{rand::{self, Rng, seq::SliceRandom}, tracing};
use pyo3::types::IntoPyDict;
use tascii::prelude::*;
use tracing::warn;

use maplit::hashmap;

//Todo: Rewrite in rust

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct CobblerConfig {
    pub kernel_args: Vec<(String, String)>,
    pub image: String,
}

impl CobblerConfig {
    pub async fn new(
        instance: dashboard::Instance,
        _host: FKey<inventory::Host>,
        mailbox_endpoint: Endpoint,
        preimage_endpoint: Endpoint,
    ) -> CobblerConfig {
        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        let image = instance
            .config
            .image
            .get(&mut transaction)
            .await
            .unwrap()
            .into_inner();

        let ci_url = format!(
            "{}/{}",
            config::settings().mailbox.external_url.clone(),
            instance.id.into_id(),
        );

        let msg_url = format!("{}/push", mailbox_endpoint.to_url());
        let preimage_url = format!("{}/push", preimage_endpoint.to_url());

        let kargs: Vec<(String, String)> = vec![
            ("post-install-cinit".to_owned(), ci_url),
            ("provision_id".to_owned(), ID::new().to_string()),
            ("inbox_target".to_owned(), msg_url),
            ("pre_image_target".to_owned(), preimage_url),
        ];

        transaction.commit().await.unwrap();

        CobblerConfig {
            kernel_args: kargs,
            image: image.cobbler_name,
        }
    }

    pub async fn new_eve_config(
        instance: dashboard::Instance,
        _host: FKey<inventory::Host>,
        selected_disk: Option<String>,
    ) -> CobblerConfig {
        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        let image = instance
            .config
            .image
            .get(&mut transaction)
            .await
            .unwrap()
            .into_inner();

            let kargs: Vec<(String, String)> = vec![
                ("eve_nuke_disks".to_owned(), "sda".to_owned()), // Wipe all drives on the host
                ("eve_install_disk".to_owned(), selected_disk.unwrap_or("sda".to_owned())), // install to first drive if not otherwise specified
                ("eve_reboot_after_install".to_owned(), "false".to_owned()), // want to turn off host instead
                // ("root".to_owned(), "/initrd.image".to_owned()),
                // ("find_boot".to_owned(), "netboot".to_owned()),
                // ("overlaytmpfs".to_owned(), "true".to_owned()),
                // ("fastboot".to_owned(), "true".to_owned()),
                // ("console".to_owned(), "tty0".to_owned()),
                // ("console".to_owned(), "ttyS0,115200n8".to_owned()),
                // ("initrd".to_owned(), "amd64.initrd.img".to_owned()),
                // ("initrd".to_owned(), "amd64.installer.img".to_owned()),
                // ("initrd".to_owned(), "amd64.initrd.bits".to_owned()),
                // ("initrd".to_owned(), "amd64.rootfs.img".to_owned()),
                // ("initrd".to_owned(), "initrd.bits".to_owned()),
                // ("initrd".to_owned(), "rootfs.img".to_owned()),
                // ("eve_soft_serial".to_owned(), generate_soft_serial(16)), // having trouble onboarding hosts with soft serials
            ];
    
        transaction.commit().await.unwrap();

        CobblerConfig {
            kernel_args: kargs,
            image: image.cobbler_name,
        }
    }
}

pub struct CobblerActions {}

impl ModuleInitializer for CobblerActions {
    fn init(py: Python<'_>) -> &PyAny {
        let config::CobblerConfig {
            address,
            url,
            username,
            password,
            api_username,
            api_password,
        } = config::settings().cobbler.clone();

        let config: HashMap<&str, String> =
            hashmap! { "url" => url, "user" => api_username, "pass" => api_password };

        let config_py: &pyo3::types::PyDict = config.into_py_dict(py);

        let cobbler = PyModule::import(py, "cobbler").expect("Expected to import cobbler.py");

        let ca = cobbler
            .getattr("new_action")
            .unwrap()
            .call1((config_py,)) // this is magic, the comma *is* necessary because of tuple conversion shenaniganery in pyo3
            .unwrap();

        ca
    }
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct CobblerSync {
    hosts_to_skip: Vec<String>, // Hostnames
}

tascii::mark_task!(CobblerSync);
impl AsyncRunnable for CobblerSync {
    type Output = bool;

    async fn run(
        &mut self,
        _context: &tascii::prelude::Context,
    ) -> Result<Self::Output, tascii::prelude::TaskError> {
        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        let host_list = inventory::Host::select()
            .run(&mut transaction)
            .await
            .expect("couldn't get all hosts");

        for host in host_list.iter() {
            let res = PythonBuilder::<CobblerActions>::command("add_system")
                .arg(serde_json::to_string(&**host).expect("Expected json to convert to string"))
                .run();
            match res {
                Ok(_v) => {
                    // no issue
                }
                Err(e) => {
                    // got an exception trying to run the action
                    let (as_str, tb_str) = Python::with_gil(|gil| {
                        let tb_str = e
                            .traceback(gil)
                            .map(|tb| {
                                tb.format()
                                    .unwrap_or("Couldn't format traceback".to_owned())
                            })
                            .unwrap_or("Couldn't get traceback".to_owned());

                        let as_str = e.to_string();

                        (as_str, tb_str)
                    });

                    warn!("Got an exception trying to run a cobbler sync call for a host, err: {as_str}, traceback: {tb_str}");
                }
            }
        }
        PythonBuilder::<CobblerActions>::command("sync")
            .run()
            .expect("couldn't sync because python broke");
        Ok(true)
    }

    fn identifier() -> tascii::task_trait::TaskIdentifier {
        TaskIdentifier::named("CobblerStartProvisionTask").versioned(1)
    }

    fn summarize(&self, id: ID) -> String {
        format!("[{id} | Cobbler Sync]")
    }

    fn timeout() -> std::time::Duration {
        std::time::Duration::from_secs_f64(120.0)
    }

    fn retry_count(&self) -> usize {
        0
    }
}

pub fn generate_soft_serial(length: usize) -> String {
    let mut rng = rand::thread_rng();

    let numbers = Vec::from_iter('0'..='9');

    let lowercase = Vec::from_iter('a'..='z');
    let uppercase = Vec::from_iter('A'..='Z');

    let inner_length = (length / 3) * 3 + 3; // div ceil

    let mut s = String::with_capacity(inner_length);

    for block in 0..(inner_length / 3) {
        let block_start = block * 3;
        let _block_end = block_start + 2;

        let mut classes = [
            numbers.as_slice(),
            lowercase.as_slice(),
            uppercase.as_slice(),
        ];

        // inefficient, but this is fine since this operation is rare
        classes.shuffle(&mut rng);

        for class in classes {
            let idx: usize = rng.gen_range(0..class.len());

            let c = class[idx];

            s.push(c);
        }
    }

    s[0..length].to_owned()
}
