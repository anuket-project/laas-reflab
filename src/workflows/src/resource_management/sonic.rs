use common::prelude::{reqwest::{Client, Response}, hyper::client::conn, tracing::{log::logger, self}};
use models::{dal::{FKey, AsEasyTransaction, web::ResultWithCode}, inventory::Switch};

use super::network::NetworkConfig;
use serde::{Serialize, Deserialize};

use std::{net::{TcpStream, ToSocketAddrs}, collections::{HashMap, HashSet}, io::{Read, Write}, path::{Path, PathBuf}};
use ssh2::Session;

// Represents a SONiC switch. Stores a client and the IP address of the switch.
#[derive(Clone)]
pub struct SonicSwitch {
    commands: Vec<String>,
    config: SonicConfig,
    session: Session,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SonicVLANMember {
    pub tagging_mode: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SonicVLAN {
    pub admin_status: String,
    pub members: HashSet<String>,
    pub vlan_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SonicConfig {
    pub CRM: serde_json::Value,
    pub DEVICE_METADATA: serde_json::Value,
    pub FEATURE: serde_json::Value,
    pub FLEX_COUNTER_TABLE: serde_json::Value,
    pub PORT: serde_json::Value,
    pub TELEMETRY: Option<serde_json::Value>,
    pub VERSIONS: serde_json::Value,

    /*
    The SONiC VLANs.
    Key is the VLAN name. 
    Value consists of whether the VLAN is up, interface members, and the VLAN ID.
    */
    pub VLAN: HashMap<String, SonicVLAN>,
    
    /*
    The SONiC VLAN members. 
    Key is interface name and vlan name delimited by a "|" character.
    Value is a struct containing a single string field, which may either be "tagged" or "untagged".
    This struct defines how the vlan behaves on this interface.
    */
    pub VLAN_MEMBER: HashMap<String, SonicVLANMember>,
}

impl SonicSwitch {
    pub fn pull_config(session: &mut Session) -> SonicConfig {
        static SONIC_CFGGEN: &'static str = "/usr/local/bin/sonic-cfggen -d --print-data";
        let mut channel = session.channel_session().unwrap();
        let mut cfg = String::new();

        channel.exec(SONIC_CFGGEN).expect("Running sonic-cfggen failed");

        channel.read_to_string(&mut cfg).expect("Failed to read config into string");

        let config: SonicConfig = serde_json::from_str(cfg.as_str()).expect("Failed to serialize SONiC config");

        config
    }
    
    pub fn push_config(&mut self) {
        tracing::warn!("Inside push config function...");
        let mut channel = self.session.channel_session().unwrap();
        let sftp = self.session.sftp().unwrap();
        let filename = "config_db.json";
        let config_json = serde_json::to_string_pretty(&self.config).expect("Failed to serialize config into JSON");        
        
        let file_path = PathBuf::from(filename);
        
        let mut file = sftp.create(file_path.as_ref()).unwrap();
        
        assert!(!config_json.is_empty());
        
        file.write_all(config_json.as_str().as_bytes()).expect("Failed to write config to remote host");
        
        std::mem::drop(file);
        std::mem::drop(sftp);
        
        channel.exec("sudo mv ~/config_db.json /etc/sonic/").expect("Failed to move config");
        
        std::mem::drop(channel);
        
        let mut channel = self.session.channel_session().unwrap();
        
        channel.exec("sudo config reload --yes").expect("Failed to run config load cmd");
    }

    // Userauth-password.
    pub fn with_user_pass_auth(addr: &str, username: &str, password: &str) -> SonicSwitch {
        let mut session = Session::new()
            .expect("Failed to create a new SSH session for SONiC switch.");
        let connection = TcpStream::connect(format!("{addr}:22"))
            .expect("Failed to open TCP stream to SONiC switch.");

        session.set_tcp_stream(connection);
        session.handshake().unwrap();
        session.userauth_password(username, password)
            .expect("SSH basic authentication failed");

        SonicSwitch {
            config: Self::pull_config(&mut session),
            session,
            commands: Vec::new(),
        }
    }
    
    /*
    Get an iterator that contains needed information to manipulate the VLAN Members and VLANs.
    */
    pub fn member_list(&self) -> impl Iterator<Item=(&String, String, String, &SonicVLANMember)> {
        self.config.VLAN_MEMBER.iter().map(|(name, member)| {
            let parsed_pair: Vec<&str> = name.split('|').take(2).collect::<Vec<&str>>();
            assert!(parsed_pair.len() == 2);
                        
            let vlan_name = (*parsed_pair.get(0).unwrap()).to_owned();
            let interface_name = (*parsed_pair.get(1).unwrap()).to_owned();
                        
            (name, vlan_name, interface_name, member)
        })
    }
    
    pub fn set_single_vlan(&mut self, vlan: i16, iface: &str, tagged: bool) {
        let vlan_name = format!("Vlan{}", vlan);
        let vlan_id = format!("{}", vlan);
        let vlan_member_name = format!("{}|{}", vlan_name, iface);
        
        let vlan = self.config.VLAN.entry(vlan_name.clone())
            .or_insert(SonicVLAN { admin_status: "up".to_owned(), members: HashSet::new(), vlan_id: vlan_id.clone() });

        if vlan.admin_status != "up" {
            vlan.admin_status = "up".to_owned();
        }

        if vlan.vlan_id != vlan_id {
            panic!("Improperly set up VLAN: {} had vlan ID {}", vlan_name, vlan.vlan_id)
        }
        
        // Mark this interface as a member of this VLAN
        vlan.members.insert(iface.to_owned());
        
        if !tagged {
            let mut to_delete = None;
            
            // Query the configuration for a VLAN member that 
            let maybe_previously_untagged_vlan = self.member_list()
                    .find(|(_, old_vlan_name, interface_name, member)| !vlan_name.eq(old_vlan_name) && interface_name.eq(iface) && member.tagging_mode == "untagged");

            if let Some((name, old_vlan_name, _, _)) = maybe_previously_untagged_vlan {
                to_delete = Some(name.to_owned());
                
                self.config.VLAN.entry(old_vlan_name)
                    .and_modify(|x| { x.members.remove(iface); });
            }
            
            if let Some(to_delete) = to_delete {
                self.config.VLAN_MEMBER.remove(to_delete.as_str());
            }
        }
        
        self.config.VLAN_MEMBER.insert(vlan_member_name, SonicVLANMember {
            tagging_mode: (if tagged { "tagged" } else { "untagged" }).to_owned()
        });
    }

    /*
    Ensures that all needed VLAN objects exist.
    */
    pub fn set_interface_vlans(&mut self, untagged_vlan: Option<i16>, tagged_vlans: Vec<i16>, iface: String) {        
        let vlan_names = tagged_vlans.iter().chain(untagged_vlan.iter()).map(|x| format!("Vlan{}", x)).collect::<Vec<String>>();
        
        // Get all VLANs on this interface
        let unmentioned_vlans = self.member_list()
            .filter_map(|(name, interface, vlan, _)| {
                    if (&iface) == (&interface) && !vlan_names.contains(&vlan) {
                        Some((name.to_owned(), vlan))
                    } else {
                        None
                    }
                })
            .collect::<Vec<(String, String)>>();
        
        for (member_key, vlan_key) in unmentioned_vlans {
            self.config.VLAN_MEMBER.remove(&member_key);
            
            assert!(self.config.VLAN.contains_key(vlan_key.as_str()));
            
            self.config.VLAN.entry(vlan_key).and_modify(|vlan| {
                vlan.members.remove(&iface);
            });
        }
        
        for vlan in tagged_vlans {
            self.set_single_vlan(vlan.to_owned(), iface.as_str(), true);
        }
        
        if let Some(vlan) = untagged_vlan {
            self.set_single_vlan(vlan, iface.as_str(), false);
        }
    }

    // Queue a command to run.
    pub fn queue<S>(&mut self, input: S) -> &mut Self
    where S: Into<String> {
        self.commands.push(input.into());
        self
    }

    pub fn run_commands(&mut self, persistent: bool) -> Result<(), String> {
        let mut channel = self.session.channel_session().map_err(|e| e.to_string())?;
        channel.shell().expect("Shell to start on SONiC");

        if persistent {
            self.commands.push("config save".to_owned());
        }

        tracing::warn!("Commands to run {:?}", self.commands);

        for cmd in self.commands.iter() {
            // let cmd = format!("sudo {}", cmd);
            // if let Err(why) = channel.exec(cmd.as_str()) {
            //     return Err(format!("Could not run command \"{}\": {}", cmd, why.to_string()))
            // }

            channel.write_all(cmd.as_bytes()).expect("Expected to write command as bytes to channel!");
            channel.write_all(b"\n").expect("Expected to write new line to channel!");
        }

        channel.send_eof().expect("Expected to send eof to channel!");
        let mut info: String = String::default();
        channel.read_to_string(&mut info).expect("Expected to read info!");
        self.commands.clear();

        Ok(())
    }

}

pub async fn sonic_run_network_task(ncfg: NetworkConfig) {
    // Get a database client so we can look up switch information.
    let mut client = models::dal::new_client()
        .await
        .expect("Did not get a database client");

    // Create a new database transaction.
    let mut transaction = client.easy_transaction()
        .await
        .expect("Did not get a new database transaction from the client");

    // Place to store a relation of switches to SONiC sessions.
    let mut switches: HashMap<FKey<Switch>, SonicSwitch> = HashMap::new();

    tracing::warn!("Printing BGS");
    for bg in ncfg.bondgroups.iter() {
        tracing::warn!("{bg:?}");
        let mut native_vlan = None;
        let mut vlans = Vec::new();

        for vlan_connection in bg.vlans.iter() {
            let vlan = vlan_connection.vlan.get(&mut transaction).await.unwrap();

            if !vlan_connection.tagged {
                assert!(
                    native_vlan.replace(vlan.vlan_id).is_none(),
                    "already had a native vlan?"
                );
            } else {
                vlans.push(vlan.vlan_id)
            }
        }

        for member in bg.member_host_ports.iter() {
            let host_port = member.get(&mut transaction).await.unwrap();

            if let Some(sp_key) = host_port.switchport.as_ref() {
                let switch_port = sp_key.get(&mut transaction)
                    .await
                    .unwrap();

                let for_switch = &switch_port.for_switch;
                let switch = for_switch.get(&mut transaction)
                    .await
                    .unwrap()
                    .into_inner();

                if  switch.switch_os.unwrap().get(&mut transaction).await.expect("Expected to get OS").os_type == "SONiC".to_string() {
                    let sanic = switches.entry(*for_switch).or_insert_with(|| {
                        // TODO: Use kex authentication
                        // here is where the error is being thrown
                        SonicSwitch::with_user_pass_auth(&switch.ip, &switch.user, &switch.pass)
                    });
                    
                    sanic.set_interface_vlans(native_vlan, vlans.clone(), adams_law(switch_port.name.clone()));
                }
            }
        }
    }

    tracing::warn!("Iterating through switches...");
    tracing::warn!("{:?}",switches.keys());
    for switch in switches.values_mut() {
        tracing::warn!("Going to push configs and run commands for the switch");
        tracing::warn!("Config: {:?}", switch.config);
        switch.push_config();
        // running the commands as persistent may overwrite the new config
        switch.run_commands(false).unwrap();
    }

}

fn adams_law(iface: String) -> String {
    match iface {
        _ if iface.contains("GigE") && iface.contains("b") => {
            let mut nums = iface.split(&['E', 'b']);
            nums.next();
            let e = i32::from_str_radix(nums.next().expect("Expected digit"), 10).expect("Expected to find a digit");
            let b = i32::from_str_radix(nums.next().expect("Expected digit"), 10).expect("Expected to find a digit");
            format!("Ethernet{}", ((e-1)*4)+b)
        },
        _ if iface.contains("GigE") => {
            let mut nums = iface.split('E');
            nums.next();
            let e = i32::from_str_radix(nums.next().expect("Expected digit"), 10).expect("Expected to find a digit");
            format!("Ethernet{}", (e-1)*4)
        },
        _ => {iface}
    }
}

mod test {
    use std::{path::{Path, PathBuf}, str::FromStr};

    use serde_json::json;
    use ssh2::Session;

    use super::{SonicSwitch, SonicConfig, adams_law};
    
    #[test]
    fn adams_law_correct_single_digit_input() {
        adams_law_test(String::from("Ethernet1"), String::from("Ethernet1"))
    }

    #[test]
    fn adams_law_correct_input_eth255() {
        adams_law_test(String::from("Ethernet255"), String::from("Ethernet255"))
    }

    #[test]
    fn adams_law_hundred_gig_e4() {
        adams_law_test(String::from("hundredGigE4"), String::from("Ethernet12"))
    }

    #[test]
    fn adams_law_hundred_gig_e10() {
        adams_law_test(String::from("hundredGigE10"), String::from("Ethernet36"))
    }

    #[test]
    fn adams_law_hundred_gig_e5b1() {
        adams_law_test(String::from("hundredGigE5b1"), String::from("Ethernet17"))
    }

    fn adams_law_test(input: String, expected: String) {
        let result = adams_law(input.clone());
        assert_eq!(expected, result, "Output is incorrect. Returned \"{}\" instead of \"{}\"", result, expected);
    }



    /*
    Instantiate a dummy switch for testing configuration edits.
    */
    fn get_dummy_switch() -> SonicSwitch {
        SonicSwitch {
            commands: Vec::new(),
            config: SonicConfig {
                CRM: serde_json::Value::Null,
                DEVICE_METADATA: serde_json::Value::Null,
                FEATURE: serde_json::Value::Null,
                FLEX_COUNTER_TABLE: serde_json::Value::Null,
                PORT: serde_json::Value::Null,
                TELEMETRY: Some(serde_json::Value::Null),
                VERSIONS: serde_json::Value::Null,
                VLAN: serde_json::from_str(r#"{
                    "Vlan3002": {
                        "members": [
                            "Ethernet0"
                        ],
                        "vlan_id": "3002",
                        "admin_status": "up"
                    },
                    "Vlan98": {
                        "members": [],
                        "vlan_id": "98",
                        "admin_status": "up"
                    },
                    "Vlan99": {
                        "members": [],
                        "vlan_id": "99",
                        "admin_status": "down"  
                    },
                    "Vlan4096": {
                        "members": [
                            "EthernetBad"
                        ],
                        "vlan_id": "4097",
                        "admin_status": "sus"
                    }
                }"#).unwrap(),
                VLAN_MEMBER: serde_json::from_str(r#"{
                    "Vlan3002|Ethernet0": {
                        "tagging_mode": "untagged"
                    }
                }"#).unwrap()
            },
            session: Session::new().unwrap(), // Create some dummy session. We don't care what this is.
        }
    }

    #[test]
    fn connect() {
        //let key_path = PathBuf::from_str("/home/glen/.ssh/id_ed25519").unwrap();
        //let keys: (Option<&Path>, &Path, Option<&str>) = (None, &key_path, None);

        let sw = SonicSwitch::with_user_pass_auth(todo!(), todo!(), todo!());
        
        // Assert that authentication was successful.
        assert!(sw.session.authenticated());
        
        // Assert something we know to be true about the SONiC config (i.e. the PORT field is an object)
        assert!(sw.config.PORT.is_object());
    }

    #[test]
    fn push_config() {
        let mut sw = SonicSwitch::with_user_pass_auth(todo!(), todo!(), todo!());
        
        sw.push_config();
    }
    
    #[test]
    fn set_single_vlan_idempotence() {
        // Get a dummy switch.
        let mut test_sw = get_dummy_switch();
        
        // Test set_single_vlan
        test_sw.set_single_vlan(3002, "Ethernet0", false);
        
        let vlan_entry = test_sw.config.VLAN.get("Vlan3002");
        let vlan_member = test_sw.config.VLAN_MEMBER.get("Vlan3002|Ethernet0");
        assert!(vlan_entry.is_some());
        assert!(vlan_member.is_some());
        
        let vlan_entry = vlan_entry.unwrap();
        let vlan_member = vlan_member.unwrap();
        assert!(vlan_entry.admin_status == "up");
        assert!(vlan_entry.members.contains("Ethernet0"));
        assert!(vlan_entry.vlan_id == "3002");
        
        assert!(vlan_member.tagging_mode == "untagged");        
    }
    
    #[test]
    fn set_single_vlan_override_mode() {
        let mut test_sw = get_dummy_switch();
        
        test_sw.set_single_vlan(3002, "Ethernet0", true);
        
        let vlan_entry = test_sw.config.VLAN.get("Vlan3002");
        let vlan_member = test_sw.config.VLAN_MEMBER.get("Vlan3002|Ethernet0");
        assert!(vlan_entry.is_some());
        assert!(vlan_member.is_some());
        
        let vlan_entry = vlan_entry.unwrap();
        let vlan_member = vlan_member.unwrap();
        assert!(vlan_entry.admin_status == "up");
        assert!(vlan_entry.members.contains("Ethernet0"));
        assert!(vlan_entry.vlan_id == "3002");
        
        assert!(vlan_member.tagging_mode == "tagged");
    }
    
    #[test]
    fn set_single_vlan_new_iface() {
        let mut test_sw = get_dummy_switch();
        
        test_sw.set_single_vlan(3002, "Ethernet4", true);
        
        let vlan_entry = test_sw.config.VLAN.get("Vlan3002");
        let vlan_member = test_sw.config.VLAN_MEMBER.get("Vlan3002|Ethernet4");
        assert!(vlan_entry.is_some());
        assert!(vlan_member.is_some());
        
        assert!(test_sw.config.VLAN_MEMBER.contains_key("Vlan3002|Ethernet0"));
        
        let vlan_entry = vlan_entry.unwrap();
        let vlan_member = vlan_member.unwrap();
        assert!(vlan_entry.admin_status == "up");
        assert!(vlan_entry.members.contains("Ethernet4"));
        assert!(vlan_entry.members.contains("Ethernet0"));
        assert!(vlan_entry.vlan_id == "3002");
        
        assert!(vlan_member.tagging_mode == "tagged");
    }
    
    #[test]
    fn set_single_vlan_new_vlan() {
        let mut test_sw = get_dummy_switch();
        
        test_sw.set_single_vlan(3004, "Ethernet0", true);
        
        let vlan_entry = test_sw.config.VLAN.get("Vlan3004");
        let vlan_member = test_sw.config.VLAN_MEMBER.get("Vlan3004|Ethernet0");
        assert!(vlan_entry.is_some());
        assert!(vlan_member.is_some());
        
        assert!(test_sw.config.VLAN_MEMBER.contains_key("Vlan3002|Ethernet0"));
        
        let vlan_entry = vlan_entry.unwrap();
        let vlan_member = vlan_member.unwrap();
        assert!(vlan_entry.admin_status == "up");
        assert!(vlan_entry.members.contains("Ethernet0"));
        assert!(vlan_entry.vlan_id == "3004");
        
        assert!(vlan_member.tagging_mode == "tagged");
    }
    
    #[test]
    fn set_single_vlan_add_new_vlan_and_iface() {
                let mut test_sw = get_dummy_switch();
        
        test_sw.set_single_vlan(3010, "Ethernet24", true);
        
        let vlan_entry = test_sw.config.VLAN.get("Vlan3010");
        let vlan_member = test_sw.config.VLAN_MEMBER.get("Vlan3010|Ethernet24");
        assert!(vlan_entry.is_some());
        assert!(vlan_member.is_some());
        
        assert!(test_sw.config.VLAN_MEMBER.contains_key("Vlan3002|Ethernet0"));
        assert!(test_sw.config.VLAN.contains_key("Vlan3002"));
        
        let vlan_entry = vlan_entry.unwrap();
        let vlan_member = vlan_member.unwrap();
        assert!(vlan_entry.admin_status == "up");
        assert!(vlan_entry.members.contains("Ethernet24"));
        assert!(vlan_entry.vlan_id == "3010");
        
        assert!(vlan_member.tagging_mode == "tagged");
    }
    
    #[test]
    fn set_single_vlan_override_untagged_vlan() {
        let mut test_sw = get_dummy_switch();
        
        test_sw.set_single_vlan(3004, "Ethernet0", false);
        
        let vlan_entry = test_sw.config.VLAN.get("Vlan3004");
        let vlan_member = test_sw.config.VLAN_MEMBER.get("Vlan3004|Ethernet0");
        assert!(vlan_entry.is_some());
        assert!(vlan_member.is_some());
        
        // We want this to be unset as a result of our action.
        assert!(!test_sw.config.VLAN_MEMBER.contains_key("Vlan3002|Ethernet0"));
        
        let vlan_entry = vlan_entry.unwrap();
        let vlan_member = vlan_member.unwrap();
        assert!(vlan_entry.admin_status == "up");
        assert!(vlan_entry.members.contains("Ethernet0"));
        assert!(vlan_entry.vlan_id == "3004");
        
        assert!(vlan_member.tagging_mode == "untagged");
    }
    
    #[test]
    fn set_interface_vlans_idempotent() {
        let mut test_sw = get_dummy_switch();
        
        let vlans = vec![];
        
        test_sw.set_interface_vlans(Some(3002), vlans, "Ethernet0".to_owned());
        
        assert!(test_sw.config.VLAN_MEMBER.contains_key("Vlan3002|Ethernet0"));
        assert!(test_sw.config.VLAN_MEMBER.get("Vlan3002|Ethernet0").unwrap().tagging_mode.eq("untagged"));
        assert!(test_sw.config.VLAN.contains_key("Vlan3002"));
        assert!(test_sw.config.VLAN.contains_key("Vlan98"));
        assert!(test_sw.config.VLAN.contains_key("Vlan99"));
    }
    
    #[test]
    fn set_interface_vlans_on_existing_interface() {
        let mut test_sw = get_dummy_switch();
        let iface = "Ethernet0";
        let vlans = vec![99, 3002];
        
        test_sw.set_interface_vlans(Some(98), vlans, iface.to_owned());
        
        assert!(test_sw.config.VLAN_MEMBER.get("Vlan3002|Ethernet0").is_some_and(|x| x.tagging_mode.eq("tagged")));
        assert!(test_sw.config.VLAN_MEMBER.get("Vlan99|Ethernet0").is_some_and(|x| x.tagging_mode.eq("tagged")));
        assert!(test_sw.config.VLAN_MEMBER.get("Vlan98|Ethernet0").is_some_and(|x| x.tagging_mode.eq("untagged")));
        assert!(test_sw.config.VLAN.get("Vlan3002").is_some_and(|x| x.members.contains(iface))); 
        assert!(test_sw.config.VLAN.get("Vlan98").is_some_and(|x| x.members.contains(iface)));
        assert!(test_sw.config.VLAN.get("Vlan99").is_some_and(|x| x.members.contains(iface)));
    }

    #[test]
    #[should_panic]
    fn set_single_vlan_panics_on_invalid_config() {
        get_dummy_switch().set_single_vlan(4096, "Ethernet0", true);
    }
}
