use dal::{new_client, AsEasyTransaction, FKey, Lookup, ID};
use eui48::MacAddress;

use models::inventory::{
    Arch, DataUnit, DataValue, Flavor, HostPort, ImportFlavor, ImportHost, Lab,
};
use serde::{Deserialize, Serialize};

use ssh2::Session;
use std::{
    fs::File,
    io::{Read, Write},
    net::TcpStream,
    path::PathBuf,
    str::FromStr,
    time::Duration,
};
use tascii::{prelude::*, task_trait::AsyncRunnable};

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct Disks {
    blockdevices: Vec<Disk>,
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct Disk {
    mountpoints: Vec<String>,
    size: String,
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct InspectHost {
    pub lab: FKey<Lab>,
    pub flavor: String,
    pub host_name: String,
    pub host_address: String,
    pub serial: String,
    pub iol_id: String,
    pub fqdn: String,
    pub ipmi_fqdn: String,
    pub ipmi_user: String,
    pub ipmi_pass: String,
    pub ipmi_mac: String,
    pub sda_uefi_entry: String,
    pub username: String,
    pub passwd: String,
    pub brand: String,
    pub model: String,
}

tascii::mark_task!(InspectHost);
impl AsyncRunnable for InspectHost {
    type Output = ();

    async fn run(&mut self, context: &Context) -> Result<Self::Output, TaskError> {
        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        let lab = self
            .lab
            .get(&mut transaction)
            .await
            .expect("Expected to find lab");

        let mut session =
            Session::new().expect("Failed to create a new SSH session for SONiC switch.");
        let connection = TcpStream::connect(format!("{}:22", self.host_address))
            .expect("Failed to open TCP stream to SONiC switch.");

        session.set_tcp_stream(connection);
        session.handshake().unwrap();
        session
            .userauth_password(self.username.as_str(), self.passwd.as_str())
            .expect("SSH basic authentication failed");

        let mut channel = session.channel_session().unwrap();

        // workaround for some intel cards
        let _ = channel.exec(
            "for f in /sys/kernel/debug/i40e/*/command; do
                sudo bash -c \"echo lldp stop > $f\"
            done",
        );

        install_deps(&session).await;

        let arch = {
            let mut channel = session.channel_session().unwrap();
            let mut res = String::new();

            channel
                .exec("lscpu | awk '$1 == \"Architecture:\" {print $2}'")
                .expect("Expected to get host arch info");
            channel
                .read_to_string(&mut res)
                .expect("Failed to read host arch info");

            Arch::from_string_fuzzy(&res).expect("Expected to get a matching arch")
        };

        let import_host = ImportHost {
            server_name: self.host_name.clone(),
            arch,
            flavor: self.flavor.clone(),
            serial: self.serial.clone(),
            ipmi_fqdn: self.ipmi_fqdn.clone(),
            iol_id: self.iol_id.clone(),
            ipmi_mac: MacAddress::parse_str(self.ipmi_mac.clone().as_str())
                .expect("Expected valid mac addr"),
            ipmi_user: self.ipmi_user.clone(),
            ipmi_pass: self.ipmi_pass.clone(),
            fqdn: self.fqdn.clone(),
            projects: vec![lab.name.clone()],
            sda_uefi_device: match self.sda_uefi_entry.as_str() {
                "" => None,
                s => Some(s.to_owned()),
            },
        };

        let conn_info = {
            let mut channel = session.channel_session().unwrap();
            let mut res = String::new();

            channel.exec("").expect("");
            channel
                .read_to_string(&mut res)
                .expect("Failed to read host connection info");
        };

        let import_connection_list: Vec<HostPort> = Vec::new();

        let cpu_count = {
            let mut channel = session.channel_session().unwrap();
            let mut res = String::new();

            channel
                .exec("lscpu | awk '$1 == \"CPU(s):\" {print $2}'")
                .expect("Expected to get host arch info");
            channel
                .read_to_string(&mut res)
                .expect("Failed to read host cpu count info");

            usize::from_str_radix(&res, 10).expect("Expected to parse")
        };

        let ram = {
            let mut channel = session.channel_session().unwrap();
            let mut res = String::new();

            channel.exec("sudo lshw -c memory -short | awk ' $4 == \"System\" {print $3}' | sed 's/iB//g'").expect("Expected to get host ram info");
            channel
                .read_to_string(&mut res)
                .expect("Failed to read host arch info");

            match res.clone() {
                s if s.contains('M') => {
                    DataValue::from_decimal(&s.split('K').next().unwrap(), DataUnit::KiloBytes)
                }
                s if s.contains('M') => {
                    DataValue::from_decimal(&s.split('M').next().unwrap(), DataUnit::MegaBytes)
                }
                s if s.contains('G') => {
                    DataValue::from_decimal(&s.split('G').next().unwrap(), DataUnit::GigaBytes)
                }
                s if s.contains('T') => {
                    DataValue::from_decimal(&s.split('T').next().unwrap(), DataUnit::TeraBytes)
                }
                _ => DataValue::from_decimal("0", DataUnit::GigaBytes),
            }
            .expect("Expected to parse")
        };

        let root_size = {
            let mut channel = session.channel_session().unwrap();
            let mut res = String::new();

            channel.exec("lsblk -n -o \"MOUNTPOINTS,SIZE\" | awk '{ printf(\"(\\\"%s\\\",\\\"%s\\\")\\\n\\\", $1, $2) }' | grep -m 1 \\\"/\\\"").expect("Expected to get host arch info");
            channel
                .read_to_string(&mut res)
                .expect("Failed to read host arch info");

            let disk_info: (&str, &str) =
                serde_json::from_str(&res).expect("Expected to deserialize disk info");
            match disk_info {
                (_, s) if s.contains('M') => {
                    DataValue::from_decimal(&s.split('K').next().unwrap(), DataUnit::KiloBytes)
                }
                (_, s) if s.contains('M') => {
                    DataValue::from_decimal(&s.split('M').next().unwrap(), DataUnit::MegaBytes)
                }
                (_, s) if s.contains('G') => {
                    DataValue::from_decimal(&s.split('G').next().unwrap(), DataUnit::GigaBytes)
                }
                (_, s) if s.contains('T') => {
                    DataValue::from_decimal(&s.split('T').next().unwrap(), DataUnit::TeraBytes)
                }
                (_, _) => DataValue::from_decimal("0", DataUnit::GigaBytes),
            }
            .expect("Expected to parse")
        };

        let disk_size = {
            let mut channel = session.channel_session().unwrap();
            let mut res = String::new();

            channel.exec("lsblk -n -o \"MOUNTPOINTS,SIZE\" | awk '{ printf(\"(\\\"%s\\\",\\\"%s\\\")\\\n\\\", $1, $2) }' | grep -m 1 ,\\\"\\\"").expect("Expected to get host arch info");
            channel
                .read_to_string(&mut res)
                .expect("Failed to read host arch info");

            let disk_info: (&str, &str) =
                serde_json::from_str(&res).expect("Expected to deserialize disk info");
            match disk_info {
                (s, _) if s.contains('K') => {
                    DataValue::from_decimal(&s.split('K').next().unwrap(), DataUnit::KiloBytes)
                }
                (s, _) if s.contains('M') => {
                    DataValue::from_decimal(&s.split('M').next().unwrap(), DataUnit::MegaBytes)
                }
                (s, _) if s.contains('G') => {
                    DataValue::from_decimal(&s.split('G').next().unwrap(), DataUnit::GigaBytes)
                }
                (s, _) if s.contains('T') => {
                    DataValue::from_decimal(&s.split('T').next().unwrap(), DataUnit::TeraBytes)
                }
                (_, _) => DataValue::from_decimal("0", DataUnit::GigaBytes),
            }
            .expect("Expected to parse")
        };

        let swap_size = {
            let mut channel = session.channel_session().unwrap();
            let mut res = String::new();

            channel.exec("lsblk -n -o \"MOUNTPOINTS,SIZE\" | awk '{ printf(\"(\\\"%s\\\",\\\"%s\\\")\\\n\\\", $1, $2) }' | grep -m 1 \\\"SWAP\\\"").expect("Expected to get host arch info");
            channel
                .read_to_string(&mut res)
                .expect("Failed to read host arch info");

            let disk_info: (&str, &str) =
                serde_json::from_str(&res).expect("Expected to deserialize disk info");
            match disk_info {
                (_, s) if s.contains('M') => {
                    DataValue::from_decimal(&s.split('K').next().unwrap(), DataUnit::KiloBytes)
                }
                (_, s) if s.contains('M') => {
                    DataValue::from_decimal(&s.split('M').next().unwrap(), DataUnit::MegaBytes)
                }
                (_, s) if s.contains('G') => {
                    DataValue::from_decimal(&s.split('G').next().unwrap(), DataUnit::GigaBytes)
                }
                (_, s) if s.contains('T') => {
                    DataValue::from_decimal(&s.split('T').next().unwrap(), DataUnit::TeraBytes)
                }
                (_, _) => DataValue::from_decimal("0", DataUnit::GigaBytes),
            }
            .expect("Expected to parse")
        };

        if Flavor::lookup(&mut transaction, vec![self.flavor.clone()])
            .await
            .is_err()
        {
            let flavor_info = ImportFlavor {
                arch,
                name: self.flavor.clone(),
                public: true,
                cpu_count,
                ram,
                root_size,
                disk_size,
                swap_size,
                brand: self.brand.clone(),
                model: self.model.clone(),
            };

            // write flavor to file too
            let mut flavor_file_path = PathBuf::from(format!(
                "./config_data/laas-hosts/inventory/flavors/{}",
                self.flavor.clone()
            ));
            flavor_file_path.set_extension("json");

            let mut flavor_file =
                File::create(flavor_file_path).expect("Expected to create flavor file");

            match flavor_file.write(
                serde_json::to_string_pretty(&flavor_info)
                    .expect("Expected to serialize flavor info to string")
                    .as_bytes(),
            ) {
                Ok(_) => Ok(()),
                Err(e) => Err(anyhow::Error::msg(format!(
                    "Failed to export flavor {} due to error {}",
                    self.flavor.clone(),
                    e.to_string()
                ))),
            }
            .expect("Expected to write flavor data");
        };

        let mut host_file_path = PathBuf::from(format!(
            "./config_data/laas-hosts/inventory/labs/{}/hosts/{}",
            lab.name.clone(),
            self.host_name.clone()
        ));
        host_file_path.set_extension("json");

        let mut host_file = File::create(host_file_path).expect("Expected to create host file");

        let host_info = serde_json::Value::from_str(
            format!(
                "{{\"host\": {}, \"connections\": {}}}",
                serde_json::to_string_pretty(&import_host)
                    .expect("Expected to convert host to string"),
                serde_json::to_string_pretty(&import_connection_list)
                    .expect("Expected to convert host ports to string")
            )
            .as_str(),
        )
        .expect("Expected to serialize host info");

        match host_file.write(
            serde_json::to_string_pretty(&host_info)
                .expect("Expected to serialize host info to string")
                .as_bytes(),
        ) {
            Ok(_) => Ok(()),
            Err(e) => Err(anyhow::Error::msg(format!(
                "Failed to export host {} due to error {}",
                self.host_name.clone(),
                e.to_string()
            ))),
        }
        .expect("Expected to write host data");

        todo!()
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("InspectHost").versioned(1)
    }

    fn summarize(&self, id: ID) -> String {
        let task_ty_name = std::any::type_name::<Self>();
        format!("Async Task {task_ty_name} with ID {id}")
    }

    fn variable_timeout(&self) -> Duration {
        Self::timeout()
    }

    fn timeout() -> Duration {
        Duration::from_secs_f64(240.0)
    }

    fn retry_count(&self) -> usize {
        0
    }
}

async fn install_deps(session: &Session) {
    let mut channel = session.channel_session().unwrap();
    let deps = "lldpd lldpctl awk lscpu free killall systemctl ethtool ip";

    if {
        let mut res = String::new();
        channel.exec("which apt").expect("Expected to find apt");
        channel
            .read_to_string(&mut res)
            .expect("Failed to read host arch info");
        !res.contains("no")
    } {
        channel.exec("apt update -y").expect("Expected to update");
        channel
            .exec(format!("apt install -y {}", deps).as_str())
            .expect("Expected to update");
    } else if {
        let mut res = String::new();
        channel
            .exec("which apt-get")
            .expect("Expected to find apt-get");
        channel
            .read_to_string(&mut res)
            .expect("Failed to read host arch info");
        !res.contains("no")
    } {
        channel
            .exec("apt-get update -y")
            .expect("Expected to update");
        channel
            .exec(format!("apt-get install -y {}", deps).as_str())
            .expect("Expected to update");
    } else if {
        let mut res = String::new();
        channel.exec("which yum").expect("Expected to find yum");
        channel.read_to_string(&mut res).expect("Failed to find");
        !res.contains("no")
    } {
        channel
            .exec(format!("yum install -y {}", deps).as_str())
            .expect("Expected to install");
    } else if {
        let mut res = String::new();
        channel.exec("which dnf").expect("Expected to find dnf");
        channel
            .read_to_string(&mut res)
            .expect("Failed to find dnf");
        !res.contains("no")
    } {
        channel
            .exec(format!("dnf install -y {}", deps).as_str())
            .expect("Expected to install");
    }
}
