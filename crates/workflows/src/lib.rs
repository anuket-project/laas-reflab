#![doc = include_str!("../README.md")]

pub mod cleanup_booking;
pub mod configure_networking;
pub mod deploy_booking;
pub mod entry;
pub mod resource_management;
pub mod users;
pub mod utils;
use mac_address::MacAddress;

use common::prelude::rand::{self, Rng, seq::SliceRandom};
use notifications::templates::render_template;

use models::inventory::HostPort;

use ::users::ipa;
use tracing::info;

use crate::{configure_networking::vlan_connection::NetworkManagerVlanConnection, resource_management::mailbox::Endpoint};

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


#[derive(serde::Serialize, Debug)]
struct MailboxPair {
    preimage_waiter: String,
    imaging_waiter: String,
}

#[derive(serde::Serialize, Debug)]
struct IpaUserFormatted {
    username: String,
    ssh_keys: Vec<String>,
}

impl IpaUserFormatted {
    fn from_users(ipa_users: Vec<ipa::User>) -> Vec<IpaUserFormatted> {
        let mut formated_ipa_users: Vec<IpaUserFormatted> = vec![];
        for user in ipa_users {
            let mut ssh_keys: Vec<String> = vec![];
            for ssh_key in user.ipasshpubkey.unwrap() {
                ssh_keys.push(ssh_key);
            }

            formated_ipa_users.push(IpaUserFormatted {
                username: user.uid,
                ssh_keys,
            });
        }

        formated_ipa_users
    } 
}

#[derive(serde::Serialize, Debug)]
struct EthernetConnectionFormatted {
    device_name: String,
    mac_addr: MacAddress
}

impl EthernetConnectionFormatted {
    fn from_ports(ports: Vec<HostPort>) -> Vec<EthernetConnectionFormatted> {
        let mut formatted_ether_connections: Vec<EthernetConnectionFormatted> = vec![];

        for port in ports {
            formatted_ether_connections.push(EthernetConnectionFormatted { 
                device_name: port.name, 
                mac_addr: port.mac 
            });
        }

        formatted_ether_connections
    }
}

#[derive(serde::Serialize, Debug)]
struct VlanConnectionFormatted {
    interface_name: String, // ie. pub0v109
    vlan_id: i16,
    device_name: String, // ie. eno49
}

impl VlanConnectionFormatted {
    fn from_nm_connections(nm_conns: Vec<configure_networking::vlan_connection::NetworkManagerVlanConnection>) -> Vec<VlanConnectionFormatted> {
        let mut formated_vlan_connections: Vec<VlanConnectionFormatted> = vec![];

        for nm_conn in nm_conns {
            let network_name = &nm_conn.network_name;
            let vlan_id = nm_conn.vlan_id;
            let connection_number = nm_conn.connection_number;
            let device_name = &nm_conn.device_name;

            let interface_name = format!("{network_name:.3}{connection_number}v{vlan_id}");

            formated_vlan_connections.push(VlanConnectionFormatted { 
                interface_name: interface_name, 
                vlan_id: vlan_id, 
                device_name: device_name.clone() 
            });

        }
        
        formated_vlan_connections
    }
}


// For RHEL
pub fn render_kickstart_template(
    // All inputs are typed when possible to ensure safety so we don't accidentally insert garbage strings
    pxe_address: String,
    base_config_uri: String, // 
    ipa_users: Vec<ipa::User>,
    interfaces: Vec<HostPort>,
    vlan_configs: Vec<String>, //Need to be the full vlan configuration strings for a kickstart file
    preimage_endpoint: Endpoint,
    postimage_endpoint: Endpoint,
) -> Result<String, tera::Error> {
    info!("Rendering Kickstart Template for RHEL based image");

    let mut formatted_interfaces: Vec<String> = vec![];
    for interface in interfaces {
        formatted_interfaces.push(interface.name);
    }

    let formatted_mailboxes: MailboxPair = MailboxPair {
        preimage_waiter: preimage_endpoint.to_url(),
        imaging_waiter: postimage_endpoint.to_url(),
    };

    let mut template_context = tera::Context::new();

    template_context.insert("pxe_address", &pxe_address);
    template_context.insert("base_config_uri", &base_config_uri);
    template_context.insert("ipa_users", &IpaUserFormatted::from_users(ipa_users));
    template_context.insert("vlan_configs", &vlan_configs); // to-do, move rendering of configs to jinja template and use local functions and variables
    template_context.insert("interfaces", &formatted_interfaces);
    template_context.insert("mailbox_endpoint", &formatted_mailboxes);

    render_template("generic/kickstart.j2", &template_context)
}



// For Ubuntu
pub fn render_autoinstall_template(
    ipa_users: Vec<ipa::User>,
    preimage_endpoint: Endpoint,
    postimage_endpoint: Endpoint,
    hostname: String,
    ports: Vec<HostPort>,
    nm_connections: Vec<NetworkManagerVlanConnection>

) -> Result<String, tera::Error> {
    info!("Rendering Cloud-init Template for Ubuntu");

    let formatted_mailboxes: MailboxPair = MailboxPair {
        preimage_waiter: preimage_endpoint.to_url(),
        imaging_waiter: postimage_endpoint.to_url(),
    };

    let mut template_context = tera::Context::new();

    template_context.insert("ipa_users", &IpaUserFormatted::from_users(ipa_users));
    template_context.insert("hostname", &hostname);
    template_context.insert("mailbox_endpoint", &formatted_mailboxes);
    template_context.insert("ethernet_interfaces", &EthernetConnectionFormatted::from_ports(ports));
    template_context.insert("vlans", &VlanConnectionFormatted::from_nm_connections(nm_connections));
    
    
    render_template("generic/autoinstall.j2", &template_context)
}