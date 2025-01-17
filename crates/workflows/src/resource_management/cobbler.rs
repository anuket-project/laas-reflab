use std::{collections::HashMap, path::Path};

use config::settings;
use dal::{new_client, AsEasyTransaction, DBTable, FKey, ID};

use models::{dashboard, inventory::{self, Host}};
use pyo3::{prelude::*, types::PyAny};
use serde::{Deserialize, Serialize};
use serde_json;

use crate::{resource_management::mailbox::Endpoint, utils::python::*};
use common::prelude::{
    rand::{self, seq::SliceRandom, Rng},
    tracing,
};
use pyo3::types::IntoPyDict;
use tascii::prelude::*;
use tracing::{info, warn};

use maplit::hashmap;

//Todo: Rewrite in rust

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct CobblerConfig {
    pub kernel_args: Vec<(String, String)>,
    pub image: String,
}

impl CobblerConfig {
    pub fn new(
        image_cobbler_name: String,
        instance_id: FKey<dashboard::Instance>,
        _host: FKey<inventory::Host>,
        mailbox_endpoint: Option<Endpoint>,
        preimage_endpoint: Option<Endpoint>,
    ) -> CobblerConfig {

        let ci_url = format!(
            "{}/{}",
            config::settings().mailbox.external_url.clone(),
            instance_id.into_id(),
        );

        let mut kargs: Vec<(String, String)> = vec![
            ("post-install-cinit".to_owned(), ci_url),
            ("provision_id".to_owned(), ID::new().to_string()),
        ];

        if let Some(mailbox_endpoint) = mailbox_endpoint {
            kargs.push(("inbox_target".to_owned(), format!("{}/push", mailbox_endpoint.to_url())));
        }

        if let Some(preimage_endpoint) = preimage_endpoint {
            kargs.push(("pre_image_target".to_owned(), format!("{}/push", preimage_endpoint.to_url())));
        }

        CobblerConfig {
            kernel_args: kargs,
            image: image_cobbler_name,
        }
    }

}

pub struct CobblerActions {}

impl ModuleInitializer for CobblerActions {
    fn init(py: Python<'_>) -> &PyAny {
        let config::CobblerConfig {
            api,
            ssh
        } = config::settings().cobbler.clone();

        let config: HashMap<&str, String> =
            hashmap! { "url" => api.url, "user" => api.username, "pass" => api.password };

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

/// Uses SFTP and SSH to override the system grub config files for a given host on cobbler.
/// This is useful for distros such as EVE-OS which require custom templated grub config files that cannot be
/// generated by cobbler directly.
///
/// # Arguments
///
/// * `host` - Host to update grub configs for
/// * `config_content` - Text content to write to the grub config file (i.e. a rendered jinja template)
///
/// # Returns
///
/// Returns [`Ok`] or [`anyhow::Error`]
///
pub async fn override_system_grub_config(host: &Host, config_content: &str) -> Result<(), anyhow::Error> {

    let mut client = new_client().await?;
    let mut transaction = client.easy_transaction().await?;

    // Get mac addresses
    let host_ports = host.ports(&mut transaction).await?;
    
    let mac_address_filenames: Vec<String> =
        host_ports
        .iter()
        .map(|host_port| format!("{}", host_port.mac).to_ascii_lowercase())
        .collect();

    // Push files to cobbler via ssh
    let cobbler = settings().cobbler.clone();
    let mut session =
        ssh2::Session::new().expect("Failed to create a new SSH session for cobbler.");
    let connection = std::net::TcpStream::connect(format!("{}:{}", cobbler.ssh.address, cobbler.ssh.port)).expect(
        format!("Failed to open TCP stream to cobbler at {}:{}.", cobbler.ssh.address, cobbler.ssh.port).as_str(),
    );


    session.set_tcp_stream(connection);
    session.handshake().unwrap();
    session
        .userauth_password(&cobbler.ssh.user, &cobbler.ssh.password)
        .expect("SSH basic authentication failed");

    let sftp = session.sftp().expect("Expected to open sftp session");

    let writable_directory = &cobbler.ssh.writable_directory; // /tmp
    let system_directory = &cobbler.ssh.system_directory; // /srv/tftpboot/grub/system

    for filename in &mac_address_filenames {

        // Cannot sftp with elevated privileges, so we need to place in an accessible directory first then move after.
        let remote_temp_path = &format!("{writable_directory}/{filename}");

        std::io::Write::write_all(&mut sftp.open_mode(
            Path::new(&remote_temp_path),
            ssh2::OpenFlags::CREATE | ssh2::OpenFlags::WRITE | ssh2::OpenFlags::TRUNCATE,
            0o644,
            ssh2::OpenType::File
        ).unwrap(), config_content.as_bytes()).unwrap();

        info!("Writing grub config to {}", &remote_temp_path);

        // Channels cannot be reused
        let mut channel = session
        .channel_session()?;

        info!("Copying grub config from {remote_temp_path} to {system_directory}");

        channel.exec(&format!("sudo cp {remote_temp_path} {system_directory}"))?;
    }

    Ok(())

}
