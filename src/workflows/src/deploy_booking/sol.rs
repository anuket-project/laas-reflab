//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use common::prelude::tracing;
use std::{
    process::{Command, Stdio},
    time::Duration,
};

use common::prelude::tokio::{self, fs::OpenOptions, io::AsyncWriteExt};
use models::{
    dal::{new_client, AsEasyTransaction, FKey},
    dashboard::{Aggregate, Instance},
    inventory::Host,
};
use tascii::prelude::*;

tascii::mark_task!(StashSOLOutput);
#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct StashSOLOutput {
    pub host: FKey<Host>,
    pub instance: Option<FKey<Instance>>,
    pub aggregate: Option<FKey<Aggregate>>,
    pub wait: Duration,
}

impl AsyncRunnable for StashSOLOutput {
    type Output = ();

    async fn run(&mut self, _context: &Context) -> Result<Self::Output, TaskError> {
        tracing::info!(
            "Waiting for {:?} until we try to start an SOL connection",
            self.wait
        );
        tokio::time::sleep(self.wait).await;

        tracing::info!("Starting SOL connection");

        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        let host = self.host.get(&mut transaction).await.unwrap().into_inner();

        transaction.commit().await.unwrap();

        let _ = Command::new("ipmitool")
            .args([
                "-I",
                "lanplus",
                "-H",
                &host.ipmi_fqdn,
                "-U",
                &host.ipmi_user,
                "-P",
                &host.ipmi_pass,
                "sol",
                "deactivate",
            ])
            .output();

        for _i in 0..10 {
            loop {
                let res = common::prelude::tokio::process::Command::new("ping")
                    .args(["-c", "1", &host.ipmi_fqdn.as_str()])
                    .output()
                    .await;

                if let Ok(res) = res {
                    if res.status.success() {
                        break;
                    }
                }
            }

            let mut child = Command::new("ipmitool")
                .args([
                    "-I",
                    "lanplus",
                    "-H",
                    &host.ipmi_fqdn,
                    "-U",
                    &host.ipmi_user,
                    "-P",
                    &host.ipmi_pass,
                    "sol",
                    "activate",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();

            let mut cstdout = child.stdout.take().unwrap();

            use std::io::Read;

            let mut file = OpenOptions::new()
                .write(true)
                .append(true)
                .create(true)
                .open(format!("{}_sol_output.txt", host.server_name))
                .await
                .unwrap();

            let mut buf = [0u8; 256];
            while let Ok(num) = cstdout.read(&mut buf) {
                let bytes = &buf[0..num];
                let _ = file.write_all(bytes).await;
                let _ = file.flush().await;
            }

            tokio::time::sleep(Duration::from_secs(15)).await;
        }

        Ok(())
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("StashSOLOutputTask").versioned(1)
    }

    fn variable_timeout(&self) -> Duration {
        self.wait + std::time::Duration::from_secs(60 * 60 * 2)
    }
}
