//! Functions to pull data from [laas-hosts](https://bitbucket.iol.unh.edu/projects/OPNFV/repos/laas-hosts/) repo
//! and import them as rows into the database. These functions are run on command as an option in the CLI.

use common::prelude::{
    chrono::Utc, config::settings, itertools::Itertools, macaddr::MacAddr6, serde_json::Value, *,
};

use std::io::Write;

use models::{
    allocation::{Allocation, AllocationReason, ResourceHandle, ResourceHandleInner},
    dal::{new_client, AsEasyTransaction, DBTable, EasyTransaction, FKey, NewRow},
    dashboard::{
        self, Aggregate, AggregateConfiguration, BondGroupConfig, BookingMetadata, Cifile,
        HostConfig, Image, Instance, LifeCycleState, Network, NetworkAssignmentMap,
        ProvisionLogEvent, Template, VlanConnectionConfig,
    },
    inventory::{
        self, Arch, CardType, DataUnit, DataValue, Flavor, Host, HostPort, IPInfo, IPNetwork,
        InterfaceFlavor, Switch, SwitchPort, Vlan,
    },
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, net::Ipv4Addr, path::PathBuf, str::FromStr};
use workflows::resource_management::allocator::{Allocator};

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
            true => models::allocation::ResourceHandleInner::PublicVlan(vlan_id),
            false => models::allocation::ResourceHandleInner::PrivateVlan(vlan_id),
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

            let rh = models::allocation::ResourceHandle::add_resource(&mut transaction, inner, lab)
                .await
                .expect("Couldn't create tracking handle for vlan");
        }
    }

    transaction.commit().await.unwrap();

    return Ok(());
}

pub async fn get_flavor(
    _session: &Server,
    mut transaction: &mut EasyTransaction<'_>,
    flavor_name: String,
) -> Result<(FKey<Flavor>, HashMap<String, FKey<InterfaceFlavor>>), anyhow::Error> {
    match Flavor::get_by_name(&mut transaction, flavor_name).await {
        Ok(v) => {
            let ports = v.ports(&mut transaction).await?;

            Ok((
                v.id,
                ports.into_iter().map(|p| (p.name.clone(), p.id)).collect(),
            ))
        }
        Err(e) => Err(anyhow::Error::msg(format!(
            "no existing flavor by that id existed, or db connection failed: {e:?}"
        ))),
    }
}

pub async fn get_or_import_flavor(
    mut session: &Server,
    mut transaction: &mut EasyTransaction<'_>,
    host_data: &Value,
    confluence_data: &Value,
    image_data: &Value,
    flavor_name: String,
    origin: String,
) -> Result<
    (
        FKey<Flavor>,
        HashMap<std::string::String, FKey<InterfaceFlavor>>,
    ),
    anyhow::Error,
> {
    match get_flavor(session, &mut transaction, flavor_name.clone()).await {
        Ok(v) => {
            writeln!(
                session,
                "Found existing flavor for host {}, flavor is named {}",
                host_data["hostname"].as_str().unwrap(),
                flavor_name
            )?;
            return Ok(v);
        }
        Err(_) => {}
    };

    writeln!(session, "Creating flavor: {}", flavor_name.clone())?;
    let mut ifaces = HashMap::new();

    let ram: String = host_data["memory"].as_str().unwrap().to_owned();
    let disk: String = host_data["disk"][0]["size"].as_str().unwrap().to_owned();

    let flavor = Flavor {
        id: FKey::new_id_dangling(),
        name: flavor_name.clone(),
        public: true,
        arch: Arch::X86_64, // TODO
        cpu_count: host_data["cpu"]["cpus"]
            .as_i64()
            .expect("Expected valid core count") as usize,
        ram: DataValue::from_decimal(&ram[0..ram.len() - 1], DataUnit::Bytes).unwrap(),
        root_size: DataValue::from_decimal(&disk[0..disk.len() - 3], DataUnit::Bytes).unwrap(),
        disk_size: DataValue {
            value: 0,
            unit: DataUnit::Bytes,
        },
        swap_size: DataValue {
            value: 0,
            unit: DataUnit::Bytes,
        },
    };

    let fk = NewRow::new(flavor.clone()).insert(&mut transaction).await?;

    for (_interface_mac, iface) in host_data["interfaces"].as_object().unwrap() {
        let iface_flav = InterfaceFlavor {
            name: iface["name"].as_str().unwrap().to_owned(),
            speed: DataValue::from_decimal(
                iface
                    .as_object()
                    .unwrap()
                    .get("speed")
                    .unwrap()
                    .as_i64()
                    .unwrap_or_else(|| {
                        writeln!(session, "Expected valid iface speed for host").unwrap();
                        10000
                    })
                    .to_string()
                    .as_str(),
                DataUnit::BitsPerSecond,
            )
            .unwrap(),
            id: FKey::new_id_dangling(),
            on_flavor: fk,
            cardtype: CardType::Unknown,
        };

        writeln!(
            session,
            "Creates interface for flavor with name {}",
            iface_flav.name
        )?;

        ifaces.insert(iface_flav.name.clone(), iface_flav.id);

        NewRow::new(iface_flav)
            .insert(&mut transaction)
            .await
            .unwrap();

        import_image_and_create_templates(
            session,
            &mut transaction,
            host_data,
            confluence_data,
            image_data,
            origin.clone(),
            ifaces.clone(),
            fk,
        )
        .await
        .expect("Expected to create images and template for host");
    }
    Ok((fk, ifaces))
}

pub async fn import_image_and_create_templates(
    mut session: &Server,
    mut transaction: &mut EasyTransaction<'_>,
    _host_data: &Value,
    _confluence_data: &Value,
    image_data: &Value,
    origin: String,
    ifaces: HashMap<String, FKey<InterfaceFlavor>>,
    fk: FKey<Flavor>,
) -> Result<(), anyhow::Error> {
    let flavor_name = fk.get(&mut transaction).await.unwrap().name.clone();
    let _ = writeln!(session, "Importing images for flavor: {flavor_name}");
    for image in image_data.as_object().unwrap() {
        for img_flavor in image.1["flavors"].as_array().unwrap() {
            if img_flavor["name"].as_str().unwrap() == flavor_name.clone() {
                match Image::get_by_name(&mut transaction, image.0.clone()).await {
                    Ok(mut v) => {
                        let _ = writeln!(
                            session,
                            "Adding flavor ({flavor_name}: {}) to {}",
                            fk.into_id().to_string(),
                            v.name
                        );
                        v.flavors.push(fk.clone());
                        v.update(&mut transaction).await.unwrap();
                    }
                    Err(_e) => {
                        let _ =
                            writeln!(session, "Did not find existing image: {}", image.0.clone());
                        let _iid = NewRow::new(Image {
                            id: FKey::new_id_dangling(),
                            owner: "admin".to_owned(),
                            name: image.0.clone(),
                            deleted: false,
                            cobbler_name: image.1["cobbler_name"].as_str().unwrap().to_owned(),
                            public: true,
                            flavors: vec![fk.clone()],
                        })
                        .insert(&mut transaction)
                        .await
                        .expect("couldn't insert image");
                    }
                };
                let net_id = FKey::new_id_dangling();

                let first_iface = ifaces
                    .get_key_value("ens1f0")
                    .unwrap_or(
                        ifaces
                            .get_key_value("enP2p1s0v0")
                            .unwrap_or(ifaces.iter().next().unwrap()),
                    )
                    .0;

                let images =
                    dashboard::Image::images_for_flavor(&mut transaction, fk.clone(), None)
                        .await
                        .expect("Expected to find images for flavor");

                let mut desc: String = "Default template for ".to_owned();
                for image in image_data.as_object().unwrap() {
                    for img_flavor in image.1["flavors"].as_array().unwrap() {
                        if img_flavor["name"].as_str().unwrap() == flavor_name.clone() {
                            desc = img_flavor["description"].as_str().unwrap().to_owned();
                        }
                    }
                }

                if Template::get_by_name(&mut transaction, flavor_name.clone())
                    .await
                    .unwrap()
                    .len()
                    == 0
                {
                    match NewRow::new(Template {
                        id: FKey::new_id_dangling(),
                        name: flavor_name.clone(),
                        deleted: false,
                        description: desc,
                        owner: None,
                        public: false,
                        networks: vec![NewRow::new(Network {
                            id: net_id.clone(),
                            name: "public".to_owned(),
                            public: true,
                        })
                        .insert(&mut transaction)
                        .await
                        .expect("Expected to insert new network")],
                        hosts: vec![HostConfig {
                            hostname: "laas-host".to_owned(),
                            flavor: fk,
                            image: images.get(0).expect("Expected to find image").id,
                            cifile: Vec::new(),
                            connections: vec![BondGroupConfig {
                                connects_to: vec![VlanConnectionConfig {
                                    network: net_id.clone(),
                                    tagged: true,
                                }]
                                .into_iter()
                                .collect(),
                                member_interfaces: vec![first_iface.clone()].into_iter().collect(),
                            }],
                        }],
                        lab: Lab::get_by_name(&mut transaction, origin.clone())
                            .await
                            .expect("Expected to find lab")
                            .expect("Expected lab to exist")
                            .id,
                    })
                    .insert(&mut transaction)
                    .await
                    {
                        Ok(_) => {
                            let _ = writeln!(session, "Created new template for flavor");
                        }
                        Err(e) => {
                            let _ = writeln!(
                                session,
                                "Error creating template for {origin} with error {}",
                                e.to_string()
                            );
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

pub async fn import_host(
    mut session: &Server,
    mut transaction: &mut EasyTransaction<'_>,
    import_path: PathBuf,
    conf_path: PathBuf,
    image_path: PathBuf,
    origin: String,
) -> Result<Host, anyhow::Error> {
    let import_json: Value =
        serde_json::from_str(&fs::read_to_string(import_path).expect("Host import not found"))
            .expect("Invalid import format.");
    let image_json: Value =
        serde_json::from_str(&fs::read_to_string(image_path).expect("Image data not found"))
            .expect("Invalid Image data format.");
    let conf_json: Value = serde_json::from_str(
        &fs::read_to_string(conf_path).expect("Host confluence data not found"),
    )
    .expect("Invalid confluence data format.");

    let name: &str = import_json["hostname"].as_str().unwrap();

    let _ = writeln!(session, "Importing host named {name}");

    let flavor = conf_json[name]["flavor"]
        .as_str()
        .ok_or(anyhow::Error::msg("no flavor given for a host"))?
        .to_owned();

    let (flavor_id, flavor_ifaces) = get_or_import_flavor(
        session,
        &mut transaction,
        &import_json,
        &conf_json,
        &image_json,
        flavor,
        origin.clone(),
    )
    .await?;

    // Handle host creation or update
    let host = Host::get_by_name(&mut transaction, name.to_owned()).await;
    let host = match host {
        Ok(mut h) => {
            h.server_name = name.to_owned();
            h.arch = Arch::from_string(import_json["cpu"]["arch"].as_str().unwrap().to_owned())
                .expect("Expected valid arch");
            h.flavor = flavor_id;
            h.serial = conf_json[name]["serial"].as_str().unwrap().to_owned();
            h.ipmi_fqdn = conf_json[name]["ipmi_fqdn"].as_str().unwrap().to_owned();
            h.iol_id = conf_json[name]["iol_id"]
                .as_str()
                .expect("Expected valid u64")
                .to_owned()
                .parse()
                .unwrap();
            h.ipmi_mac =
                models::macaddr::MacAddress::from_str(import_json["ipmi"]["mac"].as_str().unwrap())
                    .unwrap();
            h.ipmi_user = conf_json[name]["ipmi_user"].as_str().unwrap().to_owned();
            h.ipmi_pass = conf_json[name]["ipmi_pass"].as_str().unwrap().to_owned();
            h.id;
            h.fqdn = name.to_owned();

            h.update(transaction).await?;

            // TODO: need to figure out a way of re-syncing ports in this section, no good way of
            // doing this truly automatically though
            h.id
        }
        Err(_e) => {
            let host = NewRow::new(Host {
                id: FKey::new_id_dangling(),
                server_name: name.to_owned(),
                arch: Arch::from_string(import_json["cpu"]["arch"].as_str().unwrap().to_owned())
                    .expect("Expected valid arch"),
                flavor: flavor_id,
                serial: conf_json[name]["serial"].as_str().unwrap().to_owned(),
                ipmi_fqdn: conf_json[name]["ipmi_fqdn"].as_str().unwrap().to_owned(),
                iol_id: conf_json[name]["iol_id"]
                    .as_str()
                    .expect("Expected valid u64")
                    .to_owned()
                    .parse()
                    .unwrap(),
                ipmi_mac: models::macaddr::MacAddress::from_str(
                    import_json["ipmi"]["mac"].as_str().unwrap(),
                )
                .unwrap(),
                ipmi_user: conf_json[name]["ipmi_user"].as_str().unwrap().to_owned(),
                ipmi_pass: conf_json[name]["ipmi_pass"].as_str().unwrap().to_owned(),
                fqdn: name.to_owned(),
                projects: vec![origin],
            })
            .insert(&mut transaction)
            .await?;

            let lab = match Lab::get_by_name(transaction, "anuket".to_string()).await {
                Ok(o) => match o {
                    Some(lab) => lab.id,
                    None => return Err(anyhow::Error::msg("Lab does not exist".to_string())),
                },
                Err(e) => return Err(anyhow::Error::msg(e.to_string())),
            };

            models::allocation::ResourceHandle::add_resource(
                &mut transaction,
                ResourceHandleInner::Host(host),
                lab,
            )
            .await
            .expect("Expected to make host allocatable");

            for (_mac, data) in import_json["interfaces"].as_object().unwrap() {
                //
                let switch = inventory::Switch::get_by_ip(
                    &mut transaction,
                    data["switch"].as_str().unwrap().to_owned(),
                )
                .await
                .unwrap()
                .expect("Switch didnt' exist as expected");

                let portname = data["name"].as_str().unwrap().to_owned();

                let hp = HostPort {
                    id: FKey::new_id_dangling(),
                    name: portname.clone(),
                    mac: MacAddr6::from_str(data["mac"].as_str().unwrap())
                        .expect("Invalid mac addr for hostport"),
                    bus_addr: data["busaddr"].as_str().unwrap().to_owned(),
                    on_host: host,
                    speed: DataValue::from_decimal(
                        data["speed"]
                            .as_i64()
                            .unwrap_or_else(|| {
                                let _ = writeln!(session, "Expected valid iface speed for host");
                                10000
                            })
                            .to_string()
                            .as_str(),
                        DataUnit::MegaBitsPerSecond,
                    )
                    .ok_or(anyhow::Error::msg("Expected valid speed to convert"))?,
                    is_a: *flavor_ifaces
                        .get(&portname)
                        .ok_or_else(|| {
                            let _ =
                                writeln!(session,
                                "flavor_ifaces is {flavor_ifaces:?}, asked for portname {portname}"
                            );
                            panic!()
                        })
                        .unwrap(),
                    switch: switch.name.clone(),
                    switchport: Some(
                        SwitchPort::get_or_create_port(
                            transaction,
                            switch.id,
                            data["port"].as_str().unwrap().to_owned(),
                        )
                        .await?
                        .id,
                    ),
                };

                NewRow::new(hp).insert(transaction).await?;
            }

            host
        }
    };

    return Ok(host.get(&mut transaction).await.unwrap().into_inner());
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
                                Err(e) => return Err(anyhow::Error::msg(format!("Error creating switch: {}", e.to_string()))),
                            }
                        }
                    },
                    Err(e) => {
                        println!("PSQL Error: {}" ,e.to_string());
                        match create_switch_os(switch_data, &mut t).await {
                            Ok(f) => Some(f),
                            Err(e) => return Err(anyhow::Error::msg(format!("Error creating switch: {}", e.to_string()))),
                        }
                    },
                }
            },
            management_vlans: switch_data["mgmt_vlans"].as_array().expect("Expected management vlan array to exist").into_iter().map(|f|f.as_i64().expect("Expected management vlan array to contain an integer") as i16).collect_vec(),
            ipmi_vlan: switch_data["ipmi_vlan"].as_i64().expect("Expected ipmi vlan to be an integer") as i16,
            public_vlans: switch_data["public_vlans"].as_array().expect("Expected public vlan array to exist").into_iter().map(|f|f.as_i64().expect("Expected public vlan array to contain an integer") as i16).collect_vec(),
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
        Err(e) => Err(anyhow::Error::msg(format!("Failed to import or update switches: {}", e.to_string()))),
    }
}

async fn create_switch_os(switch_data: &Value, t: &mut EasyTransaction<'_>) -> Result<FKey<SwitchOS>, anyhow::Error> {
    let s_os_id = FKey::new_id_dangling();
    let s_os = SwitchOS {
        id: s_os_id.clone(),
        os_type: switch_data["type"]["name"].as_str().unwrap().to_owned(),
        version: match Version::from_string(switch_data["type"]["version"].as_str().unwrap().to_owned()) {
            Ok(v) => v,
            Err(e) => return Err(anyhow::Error::msg(e.to_string()))
        },
    };
    println!("{:?}", s_os);
    println!("{}", s_os.version.to_string());

    Ok(NewRow::new(s_os)
            .insert(t)
            .await
            .expect("couldn't insert switch_os"))
}

pub async fn import_proj(mut session: &Server, t: &mut EasyTransaction<'_>, proj: PathBuf) {
    for entry in proj.as_path().read_dir().unwrap() {
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
            let h = import_host(
                session,
                &mut transaction,
                host_file_path,
                PathBuf::from("./config_data/laas-hosts/tascii/host_confluence.json"),
                PathBuf::from("./config_data/laas-hosts/tascii/images.json"),
                proj.file_name()
                    .expect("Expected dir to be named")
                    .to_str()
                    .to_owned()
                    .unwrap()
                    .to_owned(),
            )
            .await;

            match h {
                Ok(v) => {
                    transaction.commit().await.unwrap();
                    let _ = writeln!(session, "Successfully imported {}\n", v.server_name);
                }

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
            let _ = writeln!(session, "Entry wasn't a file");
        }
    }
    let _ = writeln!(session, "Finished importing hosts");
}

pub async fn import_hosts(mut session: &Server) {
    let inven = PathBuf::from("./config_data/laas-hosts/inventory/");

    for p in inven.read_dir().unwrap() {
        let dir = p.unwrap();
        let project = dir
            .file_name()
            .to_str()
            .expect("Expected host data dir for project to have a valid name")
            .to_owned();
        let mut client = new_client().await.unwrap();
        for entry in dir.path().read_dir().unwrap() {
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
                let h = import_host(
                    session,
                    &mut transaction,
                    host_file_path,
                    PathBuf::from("./config_data/laas-hosts/tascii/host_confluence.json"),
                    PathBuf::from("./config_data/laas-hosts/tascii/images.json"),
                    project.clone(),
                )
                .await;

                match h {
                    Ok(v) => {
                        transaction.commit().await.unwrap();
                        let _ = writeln!(session, "Successfully imported {}\n", v.server_name);
                    }

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
            AllocationReason::ForBooking(),
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
        .allocate_specific_host(transaction, id, agg_id, AllocationReason::ForBooking())
        .await;

    match resp {
        Ok((vlan, _handle)) => Ok(vlan),
        Err(e) => Err(format!("error getting resource: {e:?}")),
    }
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
                sentiment: dashboard::StatusSentiment::in_progress,
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
                sentiment: dashboard::StatusSentiment::succeeded,
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
