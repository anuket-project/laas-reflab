//! Functions to pull data from [laas-hosts](https://bitbucket.iol.unh.edu/projects/OPNFV/repos/laas-hosts/) repo
//! and import them as rows into the database. These functions are run on command as an option in the CLI.

use common::prelude::{
    chrono::Utc, config::settings, itertools::Itertools, macaddr::MacAddr6, serde_json::Value, *,
};

use std::{fs::File, io::Write};

use dal::{
    new_client, AsEasyTransaction, DBTable, EasyTransaction, ExistingRow, FKey, Importable, Lookup,
    Named, NewRow, ID,
};

use models::{
    allocator::{self, Allocation, AllocationReason, ResourceHandle, ResourceHandleInner},
    dashboard::{
        self, Aggregate, AggregateConfiguration, BondGroupConfig, BookingMetadata, Cifile,
        HostConfig, Image, Instance, LifeCycleState, Network, NetworkAssignmentMap,
        ProvisionLogEvent, Template, VlanConnectionConfig,
    },
    inventory::{
        self, Arch, CardType, DataUnit, DataValue, Flavor, Host, HostPort, IPInfo, IPNetwork,
        InterfaceFlavor, Lab, Switch, SwitchOS, SwitchPort, Version, Vlan,
    },
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, net::Ipv4Addr, path::PathBuf, str::FromStr};
use workflows::resource_management::allocator::Allocator;

use crate::remote::{Select, Server};

#[derive(Serialize, Deserialize, Debug, Clone)]
struct BookingDump {
    booking_meta: BookingMeta,
    hosts: Vec<BookingHost>,
    networks: HashMap<String, BookingNetwork>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct BookingMeta {
    collaborators: Vec<String>,
    complete: bool,
    end: chrono::DateTime<Utc>,
    id: i16,
    job: String,
    lab: String,
    owner: String,
    pdf: String,
    project: String,
    purpose: String,
    start: chrono::DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct BookingHost {
    labid: String,
    name: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct BookingNetwork {
    public: bool,
    vlan_id: i16,
}

pub async fn import_vlans_once(
    mut session: &Server,
    import_path: PathBuf,
) -> Result<(), anyhow::Error> {
    let mut client = new_client().await.unwrap();
    let mut transaction = client.easy_transaction().await.unwrap();

    let vlans_file: Value =
        serde_json::from_str(&fs::read_to_string(import_path).expect("Vlan import not found"))
            .expect("Expected thing");
    let vlan_data = vlans_file["vlan_data"]
        .as_array()
        .expect("Expected vlans to be an array")
        .clone();
    let vlans: Vec<bool> = vlans_file["vlans"]
        .as_array()
        .unwrap()
        .into_iter()
        .map(|v| v.as_i64().unwrap() != 0)
        .collect();
    let reserved: Vec<bool> = vlans_file["reserved"]
        .as_array()
        .unwrap()
        .into_iter()
        .map(|v| v.as_i64().unwrap() != 0)
        .collect();

    let _block_size = 20;
    let mut data: Option<&Value> = None;

    // why are we doing this for every vlan but not using the res?
    for (count, _reserved) in vlans.clone().into_iter().enumerate() {
        let count = count as u64;

        let net = Network {
            id: FKey::new_id_dangling(),
            name: "temp import net".to_owned(),
            public: false,
        };

        let _net = NewRow::new(net).insert(&mut transaction).await.unwrap();

        let public_config = if count >= 105 && count <= 150 {
            for pub_net in vlan_data.iter().enumerate() {
                if pub_net.1["fields"]["vlan"]
                    .as_u64()
                    .expect("Expected integer")
                    == count
                {
                    data = Some(pub_net.1);
                }
            }

            // where are we getting data from
            if !data.is_some() {
                panic!("data is none!")
            }

            let data: Vec<&str> = data.unwrap()["fields"]["cidr"]
                .as_str()
                .expect("Expected string")
                .split('/')
                .collect();
            let base = Ipv4Addr::from_str(data[0]).expect("Expected a valid IPv4 address"); // huh
            let mut gateway_v4_octets = base.octets();
            gateway_v4_octets[3] = 1; // what
            let gateway = Ipv4Addr::from(gateway_v4_octets);
            let mask = u8::from_str(data[1]).expect("Expected a u8");
            Some(IPNetwork {
                v4: Some(IPInfo {
                    gateway: Some(gateway),
                    netmask: mask,
                    subnet: base,
                    provides_dhcp: true,
                }),
                v6: None,
            })
        } else {
            None
        };

        let vlan = Vlan {
            // TODO: id for this should not be i16, the vlan id itself
            // should be distinct from the object id (could later have non-unified vlan ranges across diff projects)
            id: FKey::new_id_dangling(),
            vlan_id: i16::try_from(count).expect("overlong count"),
            public_config: public_config.clone(),
        };

        let res = NewRow::new(vlan.clone()).insert(&mut transaction).await; //vlan_conn.insert_one(vlan.clone(), None);

        let vlan_id = match res {
            Err(e) => {
                writeln!(session, "Failed due to error: {:#?}", e)?;
                return Err(anyhow::Error::msg("failed to import"));
            }
            Ok(in_res) => {
                //tracing::info
                writeln!(session, "Imported vlan {}", count)?;
                in_res
            }
        };

        let inner = match public_config.is_some() {
            true => allocator::ResourceHandleInner::PublicVlan(vlan_id),
            false => allocator::ResourceHandleInner::PrivateVlan(vlan_id),
        };

        let disallowed_vlans = [0];

        // only add a handle for it if it isn't disallowed, and isn't a reserved (lab) vlan
        // this is a bad hack because prod LaaS models reserved vlans in a broken way
        if !disallowed_vlans.contains(&count)
            && (!reserved.get(count as usize).expect("weird count idx") || public_config.is_some())
        {
            let lab = match Lab::get_by_name(&mut transaction, "anuket".to_string()).await {
                Ok(o) => match o {
                    Some(lab) => lab.id,
                    None => return Err(anyhow::Error::msg("Lab not found")),
                },
                Err(e) => return Err(anyhow::Error::msg(format!("Failed to get lab: {e}"))),
            };

            let rh = allocator::ResourceHandle::add_resource(&mut transaction, inner, lab)
                .await
                .expect("Couldn't create tracking handle for vlan");
        }
    }

    transaction.commit().await.unwrap();

    return Ok(());
}

pub async fn import_switches(mut session: &Server, path: PathBuf) -> Result<(), anyhow::Error> {
    let mut client = new_client().await?;
    let mut transaction = client.easy_transaction().await?;

    let switch_json: Value = serde_json::from_str(
        &fs::read_to_string(path.as_path()).expect("Switch import data not found"),
    )
    .expect("Invalid format.");

    for (_switch, switch_data) in switch_json.as_object().unwrap() {
        let mut t = transaction.transaction().await.unwrap();
        let s = Switch {
            id: FKey::new_id_dangling(),
            name: switch_data["name"].as_str().unwrap().to_owned(),
            ip: switch_data["ip"].as_str().unwrap().to_owned(),
            user: switch_data["user"].as_str().unwrap().to_owned(),
            pass: switch_data["pass"].as_str().unwrap().to_owned(),
            switch_os: {
                let res = SwitchOS::select()
                    .where_field("os_type")
                    .equals(switch_data["type"]["name"].as_str().unwrap().to_owned())
                    .where_field("version")
                    .equals(switch_data["type"]["version"].as_str().unwrap().to_owned())
                    .run(&mut t)
                    .await;

                match res {
                    Ok(r) => {
                        println!("len: {}", r.len());
                        println!("{:?}", r);
                        if (r.len() > 0) {
                            Some(r.get(0).expect("Expected vector length to not be 0").id)
                        } else {
                            match create_switch_os(switch_data, &mut t).await {
                                Ok(f) => Some(f),
                                Err(e) => {
                                    return Err(anyhow::Error::msg(format!(
                                        "Error creating switch: {}",
                                        e.to_string()
                                    )))
                                }
                            }
                        }
                    }
                    Err(e) => {
                        println!("PSQL Error: {}", e.to_string());
                        match create_switch_os(switch_data, &mut t).await {
                            Ok(f) => Some(f),
                            Err(e) => {
                                return Err(anyhow::Error::msg(format!(
                                    "Error creating switch: {}",
                                    e.to_string()
                                )))
                            }
                        }
                    }
                }
            },
            management_vlans: switch_data["mgmt_vlans"]
                .as_array()
                .expect("Expected management vlan array to exist")
                .into_iter()
                .map(|f| {
                    f.as_i64()
                        .expect("Expected management vlan array to contain an integer")
                        as i16
                })
                .collect_vec(),
            ipmi_vlan: switch_data["ipmi_vlan"]
                .as_i64()
                .expect("Expected ipmi vlan to be an integer") as i16,
            public_vlans: switch_data["public_vlans"]
                .as_array()
                .expect("Expected public vlan array to exist")
                .into_iter()
                .map(|f| {
                    f.as_i64()
                        .expect("Expected public vlan array to contain an integer")
                        as i16
                })
                .collect_vec(),
        };

        let res =
            Switch::get_by_name(&mut t, switch_data["name"].as_str().unwrap().to_owned()).await?;

        match res {
            Some(mut v) => {
                let _ = writeln!(
                    session,
                    "Updating {}",
                    switch_data["name"].as_str().unwrap().to_owned()
                );

                *v = Switch { id: v.id, ..s };

                v.update(&mut t).await?;
                t.commit().await.unwrap();
            }
            None => {
                let _ = writeln!(
                    session,
                    "Creating {}",
                    switch_data["name"].as_str().unwrap().to_owned()
                );
                NewRow::new(s)
                    .insert(&mut t)
                    .await
                    .expect("couldn't insert switch");

                t.commit().await.unwrap();
            }
        }
    }
    match transaction.commit().await {
        Ok(r) => Ok(r),
        Err(e) => Err(anyhow::Error::msg(format!(
            "Failed to import or update switches: {}",
            e.to_string()
        ))),
    }
}

async fn create_switch_os(
    switch_data: &Value,
    t: &mut EasyTransaction<'_>,
) -> Result<FKey<SwitchOS>, anyhow::Error> {
    let s_os_id = FKey::new_id_dangling();
    let s_os = SwitchOS {
        id: s_os_id.clone(),
        os_type: switch_data["type"]["name"].as_str().unwrap().to_owned(),
        version: match Version::from_str(switch_data["type"]["version"].as_str().unwrap()) {
            Ok(v) => v,
            Err(e) => return Err(anyhow::Error::msg(e.to_string())),
        },
    };
    println!("{:?}", s_os);
    println!("{}", s_os.version.to_string());

    Ok(NewRow::new(s_os)
        .insert(t)
        .await
        .expect("couldn't insert switch_os"))
}

pub async fn import_proj_hosts(mut session: &Server, t: &mut EasyTransaction<'_>, proj: PathBuf) {
    let mut hosts_dir = proj.clone();
    hosts_dir.push("hosts");

    for entry in hosts_dir.as_path().read_dir().unwrap() {
        let mut transaction = t
            .transaction()
            .await
            .expect("Expected to get a child transaction");
        let entry = entry.unwrap();
        if entry.path().is_file()
            && entry
                .path()
                .extension()
                .unwrap()
                .to_str()
                .unwrap_or("bad")
                .contains("json")
        {
            //panic!("We had a host");
            let host_file_path = entry.path();
            let h = Host::import(&mut transaction, host_file_path, Some(proj.clone())).await;

            match h {
                Ok(o) => match o {
                    Some(e) => {
                        transaction.commit().await.unwrap();
                        let _ = writeln!(session, "Successfully imported {}\n", e.server_name);
                    }
                    None => {
                        let _ = writeln!(
                            session,
                            "Error importing {:?}, host not found\n",
                            entry.path()
                        );
                        transaction.rollback().await.unwrap();
                    }
                },

                Err(e) => {
                    let _ = writeln!(
                        session,
                        "Error importing {:?}, got error {e:?}\n",
                        entry.path()
                    );
                    transaction.rollback().await.unwrap();
                }
            }
        } else {
            let _ = writeln!(session, "Entry '{:?}' wasn't a file", entry.path());
        }
    }
    let _ = writeln!(session, "Finished importing hosts");
}

pub async fn export_hosts(mut session: &Server, t: &mut EasyTransaction<'_>) {
    let inven = PathBuf::from("./config_data/laas-hosts/inventory/labs");

    for p in inven.read_dir().unwrap() {
        let proj_dir = p.unwrap();
        export_proj_hosts(
            session,
            t,
            proj_dir
                .path()
                .file_name()
                .expect("Expected to get filename")
                .to_str()
                .expect("Expected to convert filename to string slice")
                .to_string(),
        )
        .await;
    }
    let _ = writeln!(session, "Finished exporting hosts");
}

pub async fn import_hosts(mut session: &Server) {
    let inven = PathBuf::from("./config_data/laas-hosts/inventory/labs");

    for p in inven.read_dir().unwrap() {
        let proj_dir = p.unwrap();
        let mut client = new_client().await.unwrap();
        for entry in proj_dir.path().read_dir().unwrap() {
            let entry = entry.unwrap();
            if entry.path().is_file()
                && entry
                    .path()
                    .extension()
                    .unwrap()
                    .to_str()
                    .unwrap_or("bad")
                    .contains("json")
            {
                let mut transaction = client.easy_transaction().await.unwrap();
                //panic!("We had a host");
                let host_file_path = entry.path();
                let h = Host::import(&mut transaction, host_file_path, Some(proj_dir.path())).await;

                match h {
                    Ok(o) => match o {
                        Some(h) => {
                            transaction.commit().await.unwrap();
                            let _ = writeln!(session, "Successfully imported {}\n", h.server_name);
                        }
                        None => {
                            let _ = writeln!(
                                session,
                                "Error importing {:?}, host not found\n",
                                entry.path()
                            );
                            transaction.rollback().await.unwrap();
                        }
                    },

                    Err(e) => {
                        let _ = writeln!(
                            session,
                            "Error importing {:?}, got error {e:?}\n",
                            entry.path()
                        );
                        transaction.rollback().await.unwrap();
                        //return;
                    }
                }
            } else {
                let _ = writeln!(session, "Entry wasn't a file");
            }
        }
    }
    let _ = writeln!(session, "Finished importing hosts");
}

pub async fn export_proj_hosts(mut session: &Server, t: &mut EasyTransaction<'_>, proj: String) {
    let handles = ResourceHandle::select()
        .where_field("lab")
        .equals(proj)
        .where_field("tracks_resource_type")
        .equals("host")
        .run(t)
        .await
        .expect("expected to find handles");
    for handle in handles {
        let mut transaction = t
            .transaction()
            .await
            .expect("Expected to get a child transaction");

        let host = match handle.tracks {
            ResourceHandleInner::Host(h) => {
                h.get(&mut transaction).await.expect("Expected to get host")
            }
            _ => panic!("Query returned unexpected resource type!!"),
        };

        match host.export(&mut transaction).await {
            Ok(_) => {
                transaction.commit().await.unwrap();
                let _ = writeln!(session, "Successfully exported {}\n", host.server_name);
            }
            Err(e) => {
                let _ = writeln!(
                    session,
                    "Error exporting {}, got error {e:?}\n",
                    host.server_name
                );
                transaction.rollback().await.unwrap();
            }
        }
    }
    let _ = writeln!(session, "Finished exporting hosts");
}

pub async fn import_vlans(mut session: &Server) -> Result<(), anyhow::Error> {
    let vlans = PathBuf::from("./config_data/laas-hosts/tascii/vlans.json");

    if vlans.is_file() {
        if let Ok(()) = import_vlans_once(session, vlans).await {
            writeln!(session, "Successfully imported vlans!")?;
            Ok(())
        } else {
            writeln!(session, "Failed to import!")?;
            Err(anyhow::Error::msg("failed to import"))
        }
    } else {
        writeln!(session, "Not a file!")?;
        Err(anyhow::Error::msg("failed to import, not a file"))
    }
}

async fn import_allocate_vlan(
    mut session: &Server,
    mut transaction: &mut EasyTransaction<'_>,
    agg_id: FKey<Aggregate>,
    id: FKey<Vlan>,
) -> Result<FKey<Vlan>, String> {
    writeln!(session, "Allocating vlan: {}", id.into_id()).unwrap();
    let allocator = Allocator::instance();
    let resp = allocator
        .allocate_vlan(
            &mut transaction,
            Some(agg_id),
            Some(id),
            true, // THIS BOOLEAN IS IGNORED BECAUSE WERE ALLOCATING A SPECIFIC ONE
            AllocationReason::ForBooking,
        )
        .await;

    match resp {
        Ok((vlan, _handle)) => Ok(vlan),
        Err(e) => Err(format!("error getting resource: {e:?}")),
    }
}

async fn allocate_host(
    mut session: &Server,
    transaction: &mut EasyTransaction<'_>,
    agg_id: FKey<Aggregate>,
    id: FKey<Host>,
) -> Result<FKey<Host>, String> {
    writeln!(session, "Allocating host: {}", id.into_id()).unwrap();
    let allocator = Allocator::instance();
    let resp = allocator
        .allocate_specific_host(transaction, id, agg_id, AllocationReason::ForBooking)
        .await;

    match resp {
        Ok((vlan, _handle)) => Ok(vlan),
        Err(e) => Err(format!("error getting resource: {e:?}")),
    }
}

pub async fn export_proj_templates(
    mut session: &Server,
    t: &mut EasyTransaction<'_>,
    proj: String,
) {
    let mut lab_vec = Lab::select()
        .where_field("name")
        .equals(proj)
        .run(t)
        .await
        .expect("Expected to query for lab");
    let lab = match lab_vec.len() {
        0 => {
            panic!("No labs found")
        }
        1 => lab_vec.pop().expect("Expected to find lab"),
        _ => {
            panic!("Too many labs found, got: {lab_vec:?}")
        }
    };

    for template in Template::select()
        .where_field("public")
        .equals(true)
        .where_field("lab")
        .equals(lab.id)
        .run(t)
        .await
        .expect("Expected to query for templates")
    {
        let mut transaction = t
            .transaction()
            .await
            .expect("Expected to get a child transaction");

        match template.export(&mut transaction).await {
            Ok(_) => {
                transaction.commit().await.unwrap();
                let _ = writeln!(session, "Successfully exported {}\n", template.name);
            }
            Err(e) => {
                let _ = writeln!(
                    session,
                    "Error exporting {}, got error {e:?}\n",
                    template.name
                );
                transaction.rollback().await.unwrap();
            }
        }
    }
    let _ = writeln!(session, "Finished exporting templates");
}

pub async fn export_templates(mut session: &Server, t: &mut EasyTransaction<'_>) {
    let inven = PathBuf::from("./config_data/laas-hosts/inventory/labs");

    for p in inven.read_dir().unwrap() {
        let proj_dir = p.unwrap();
        export_proj_templates(
            session,
            t,
            proj_dir
                .path()
                .file_name()
                .expect("Expected to get filename")
                .to_str()
                .expect("Expected to convert filename to string slice")
                .to_string(),
        )
        .await;
    }
    let _ = writeln!(session, "Finished exporting templates");
}

pub async fn import_proj_templates(
    mut session: &Server,
    t: &mut EasyTransaction<'_>,
    proj: PathBuf,
) {
    let mut template_dir = proj.clone();
    template_dir.push("templates");

    for entry in template_dir.as_path().read_dir().unwrap() {
        let mut transaction = t
            .transaction()
            .await
            .expect("Expected to get a child transaction");
        let entry = entry.unwrap();
        if entry.path().is_file()
            && entry
                .path()
                .extension()
                .unwrap()
                .to_str()
                .unwrap_or("bad")
                .contains("json")
        {
            //panic!("We had a template");
            let template_file_path = entry.path();
            let t =
                Template::import(&mut transaction, template_file_path, Some(proj.clone())).await;

            match t {
                Ok(o) => match o {
                    Some(e) => {
                        transaction.commit().await.unwrap();
                        let _ = writeln!(session, "Successfully imported {}\n", e.name);
                    }
                    None => {
                        let _ = writeln!(
                            session,
                            "Error importing {:?}, template not found\n",
                            entry.path()
                        );
                        transaction.rollback().await.unwrap();
                    }
                },

                Err(e) => {
                    let _ = writeln!(
                        session,
                        "Error importing {:?}, got error {e:?}\n",
                        entry.path()
                    );
                    transaction.rollback().await.unwrap();
                }
            }
        } else {
            let _ = writeln!(session, "Entry '{:?}' wasn't a file", entry.path());
        }
    }
    let _ = writeln!(session, "Finished importing templates");
}

pub async fn import_templates(mut session: &Server) {
    let inven = PathBuf::from("./config_data/laas-hosts/inventory/labs");

    for p in inven.read_dir().unwrap() {
        let proj_dir = p.unwrap();
        let mut client = new_client().await.unwrap();
        for entry in proj_dir.path().read_dir().unwrap() {
            let entry = entry.unwrap();
            if entry.path().is_file()
                && entry
                    .path()
                    .extension()
                    .unwrap()
                    .to_str()
                    .unwrap_or("bad")
                    .contains("json")
            {
                let mut transaction = client.easy_transaction().await.unwrap();
                //panic!("We had a template");
                let template_file_path = entry.path();
                let t =
                    Template::import(&mut transaction, template_file_path, Some(proj_dir.path()))
                        .await;

                match t {
                    Ok(o) => match o {
                        Some(t) => {
                            transaction.commit().await.unwrap();
                            let _ = writeln!(session, "Successfully imported {}\n", t.name);
                        }
                        None => {
                            let _ = writeln!(
                                session,
                                "Error importing {:?}, template not found\n",
                                entry.path()
                            );
                            transaction.rollback().await.unwrap();
                        }
                    },

                    Err(e) => {
                        let _ = writeln!(
                            session,
                            "Error importing {:?}, got error {e:?}\n",
                            entry.path()
                        );
                        transaction.rollback().await.unwrap();
                        //return;
                    }
                }
            } else {
                let _ = writeln!(session, "Entry wasn't a file");
            }
        }
    }
    let _ = writeln!(session, "Finished importing templates");
}

pub async fn import_images(mut session: &Server) {
    let inven = PathBuf::from("./config_data/laas-hosts/inventory/images");

    for entry in inven.read_dir().expect("Expected to read dir") {
        let mut client = new_client().await.unwrap();
        let entry = entry.unwrap();
        if entry.path().is_file()
            && entry
                .path()
                .extension()
                .unwrap()
                .to_str()
                .unwrap_or("bad")
                .contains("json")
        {
            let mut transaction = client.easy_transaction().await.unwrap();
            //panic!("We had a image");
            let image_file_path = entry.path();
            let t = Image::import(&mut transaction, image_file_path, None).await;

            match t {
                Ok(o) => match o {
                    Some(t) => {
                        transaction.commit().await.unwrap();
                        let _ = writeln!(session, "Successfully imported {}\n", t.name);
                    }
                    None => {
                        let _ = writeln!(
                            session,
                            "Error importing {:?}, image not found\n",
                            entry.path()
                        );
                        transaction.rollback().await.unwrap();
                    }
                },

                Err(e) => {
                    let _ = writeln!(
                        session,
                        "Error importing {:?}, got error {e:?}\n",
                        entry.path()
                    );
                    transaction.rollback().await.unwrap();
                    //return;
                }
            }
        } else {
            let _ = writeln!(session, "Entry wasn't a file");
        }
    }
    let _ = writeln!(session, "Finished importing images");
}

pub async fn import_bookings(mut session: &Server, booking_path: PathBuf) {
    let mut client = new_client().await.unwrap();
    let mut transaction = client.easy_transaction().await.unwrap();

    let booking_data: Vec<BookingDump> =
        serde_json::from_str(&fs::read_to_string(booking_path).expect("Booking dump not found"))
            .expect("Expected thing");

    let origin = Select::new(
        "select originating project:",
        settings()
            .projects
            .keys()
            .cloned()
            .into_iter()
            .collect_vec(),
    )
    .prompt(session)
    .unwrap();

    for old_booking in booking_data {
        let mut networks = Vec::new();

        for network in old_booking.networks.clone() {
            let nid = NewRow::new(Network {
                id: FKey::new_id_dangling(),
                name: network.0,
                public: network.1.public,
            })
            .insert(&mut transaction)
            .await
            .expect("Expected to insert new network");

            networks.push(nid);
        }

        let mut template_hosts: Vec<HostConfig> = Vec::new();

        for h in old_booking.hosts.clone() {
            let _ = writeln!(session, "host: {h:#?}");
            let host = Host::get_by_name(&mut transaction, h.name.clone())
                .await
                .expect("Expected to find specified host");
            template_hosts.push(HostConfig {
                hostname: h.name,
                flavor: host.flavor,
                image: dashboard::Image::images_for_flavor(&mut transaction, host.flavor, None)
                    .await
                    .unwrap()
                    .get(0)
                    .unwrap()
                    .id,
                cifile: Vec::new(),
                connections: Vec::new(),
            });
        }

        let template = match NewRow::new(Template {
            id: FKey::new_id_dangling(),
            name: format!(
                "{}: {}",
                old_booking.booking_meta.project, old_booking.booking_meta.job
            ),
            deleted: false,
            description: old_booking.booking_meta.purpose.clone(),
            owner: None,
            public: false,
            networks: networks.clone(),
            hosts: template_hosts,
            lab: Lab::get_by_name(&mut transaction, origin.clone())
                .await
                .expect("Expected to find lab")
                .expect("Expected lab to exist")
                .id,
        })
        .insert(&mut transaction)
        .await
        {
            Ok(fk) => {
                let _ = writeln!(session, "Created new template for booking");
                fk
            }
            Err(e) => panic!("Error creating template for booking: {}", e.to_string()),
        };

        let booking_id = FKey::new_id_dangling();

        let lc = if old_booking.booking_meta.end < Utc::now() {
            LifeCycleState::Done
        } else {
            LifeCycleState::Active
        };

        let aggregate = Aggregate {
            lab: Lab::get_by_name(&mut transaction, origin.clone())
                .await
                .expect("Expected to find lab")
                .expect("Expected lab to exist")
                .id,
            state: lc,
            id: booking_id,
            configuration: AggregateConfiguration {
                ipmi_username: String::new(),
                ipmi_password: String::new(),
            },
            deleted: if old_booking.booking_meta.end < Utc::now() {
                true
            } else {
                false
            },
            users: old_booking
                .booking_meta
                .collaborators
                .clone()
                .into_iter()
                .chain(vec![old_booking.booking_meta.owner.clone()].into_iter())
                .collect(),
            vlans: NewRow::new(NetworkAssignmentMap {
                id: FKey::new_id_dangling(),
                networks: HashMap::new(),
            })
            .insert(&mut transaction)
            .await
            .unwrap(),
            template,
            metadata: BookingMetadata {
                booking_id: Some(old_booking.booking_meta.id.to_string()),
                owner: Some(old_booking.booking_meta.owner),
                lab: Some(old_booking.booking_meta.lab.clone()),
                purpose: Some(old_booking.booking_meta.purpose.clone()),
                project: Some(old_booking.booking_meta.project.clone()),
                start: Some(old_booking.booking_meta.start),
                end: Some(old_booking.booking_meta.end),
            },
        };

        let agg = NewRow::new(aggregate)
            .insert(&mut transaction)
            .await
            .expect("Expected to create aggregate");

        let agg = agg.get(&mut transaction).await.unwrap();

        for h in old_booking.hosts.clone() {
            let host = Host::get_by_name(&mut transaction, h.name.clone())
                .await
                .expect("Expected to find specified host");
            let inst_id: FKey<Instance> = FKey::new_id_dangling();
            let inst = Instance {
                metadata: HashMap::new(),
                linked_host: Some(host.id),
                id: inst_id,
                aggregate: agg.id,
                within_template: template,
                config: HostConfig {
                    hostname: h.name.clone(),
                    flavor: host.flavor,
                    image: dashboard::Image::images_for_flavor(&mut transaction, host.flavor, None)
                        .await
                        .unwrap()
                        .get(0)
                        .unwrap()
                        .id,
                    cifile: vec![NewRow::new(Cifile {
                        id: FKey::new_id_dangling(),
                        priority: 1,
                        data: "".to_owned(),
                    })
                    .insert(&mut transaction)
                    .await
                    .unwrap()],
                    connections: Vec::new(),
                },
                network_data: NewRow::new(NetworkAssignmentMap::empty())
                    .insert(&mut transaction)
                    .await
                    .unwrap(),
            };

            let inst_fk = NewRow::new(inst)
                .insert(&mut transaction)
                .await
                .expect("couldn't insert instance");

            NewRow::new(ProvisionLogEvent {
                id: FKey::new_id_dangling(),
                instance: inst_fk,
                time: old_booking.booking_meta.start.clone(),
                prov_status: dashboard::ProvEvent::new(
                    "Provisioning",
                    "<no detail, imported event>",
                ),
                sentiment: dashboard::StatusSentiment::InProgress,
            })
            .insert(&mut transaction)
            .await
            .unwrap();

            NewRow::new(ProvisionLogEvent {
                id: FKey::new_id_dangling(),
                instance: inst_fk,
                time: chrono::Utc::now(),
                prov_status: dashboard::ProvEvent::new(
                    "Successfully Provisioned",
                    "<no detail, imported event>",
                ),
                sentiment: dashboard::StatusSentiment::Succeeded,
            })
            .insert(&mut transaction)
            .await
            .unwrap();
        }

        let mut network_keys = networks.into_iter();
        let mut vlans = agg.vlans.get(&mut transaction).await.unwrap();

        if old_booking.booking_meta.end > Utc::now() {
            for h in old_booking.hosts.clone() {
                let host = Host::get_by_name(&mut transaction, h.name.clone())
                    .await
                    .expect("Expected to find specified host");

                let _host = allocate_host(session, &mut transaction, agg.id, host.id)
                    .await
                    .unwrap();
            }

            for n in old_booking.networks {
                if n.1.vlan_id >= 0 {
                    let vlan_fk = Vlan::select()
                        .where_field("vlan_id")
                        .equals(n.1.vlan_id as i16)
                        .run(&mut transaction)
                        .await
                        .unwrap()
                        .get(0)
                        .cloned()
                        .ok_or(anyhow::Error::msg("couldn't find vlan by that vlan_id"));

                    let vlan_fk = match vlan_fk {
                        Ok(v) => v,
                        Err(e) => {
                            let _ = writeln!(session, "{e:?}");
                            continue;
                        }
                    };

                    let mut t = transaction.easy_transaction().await.unwrap();
                    let res = import_allocate_vlan(session, &mut t, agg.id, vlan_fk.id).await;
                    if let Ok(vid) = res {
                        t.commit().await.unwrap();
                        vlans.add_assignment(network_keys.next().unwrap(), vid);
                        vlans.update(&mut transaction).await.unwrap();
                    } else {
                        let _ = t.rollback().await;
                        let _ = writeln!(
                            session,
                            "Error: vlan {} was already allocated, \
                                       could not give it to booking with purpose {} and project {}",
                            n.1.vlan_id,
                            old_booking.booking_meta.purpose,
                            old_booking.booking_meta.project,
                        );

                        let rh = match ResourceHandle::handle_for_vlan(&mut transaction, vlan_fk.id)
                            .await
                        {
                            Ok(v) => v,
                            Err(_e) => {
                                let _ = writeln!(
                                    session,
                                    "couldn't find handle for vlan id {:?}",
                                    n.1.vlan_id
                                );
                                continue;
                            }
                        };

                        let currently_in = Allocation::find(&mut transaction, rh.id, false)
                            .await
                            .unwrap();

                        for a in currently_in {
                            if let Some(agg) = a.for_aggregate {
                                let agg = agg.get(&mut transaction).await.unwrap();

                                let _ = writeln!(
                                    session,
                                    "Vlan is currently within aggregate {:?}, {:?}, {:?}",
                                    agg.metadata.purpose, agg.metadata.project, agg.id
                                );
                            } else {
                                let _ = writeln!(session, "In an allocation with no aggregate");
                            }
                        }
                    }
                }
            }
        }

        let _ = writeln!(
            session,
            "Created booking for: {}\n",
            old_booking.booking_meta.purpose
        );
    }
    let _ = match transaction.commit().await {
        Ok(_) => writeln!(session, "Successfully imported bookings!"),
        Err(e) => writeln!(
            session,
            "Failed to import bookings due to: {}\n",
            e.to_string()
        ),
    };
}
