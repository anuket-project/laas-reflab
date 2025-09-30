#![doc = include_str!("../README.md")]
#![feature(result_flattening, iter_intersperse, if_let_guard)]

pub mod cleanup_booking;
pub mod configure_networking;
pub mod deploy_booking;
pub mod entry;
pub mod resource_management;
pub mod users;
pub mod utils;

use notifications::templates::render_template;

use models::inventory::HostPort;

use ::users::ipa;
use tracing::info;

use crate::resource_management::mailbox::Endpoint;

pub fn render_kickstart_template(
    // All inputs are typed when possible to ensure safety so we don't accidentally insert garbage strings
    iso_directory_name: String, // eg "fedora-42-1.1-x86_64"
    ipa_users: Vec<ipa::User>,
    interfaces: Vec<HostPort>,
    vlan_configs: Vec<String>, //Need to be the full vlan configuration strings for a kickstart file
    preimage_endpoint: Endpoint,
    postimage_endpoint: Endpoint,
) -> Result<String, tera::Error> {
    info!("Rendering Kickstart Template for Fedora");

    #[derive(serde::Serialize)]
    struct MailboxPair {
        preimage_waiter: String,
        imaging_waiter: String,
    }
    #[derive(serde::Serialize, Debug)]
    struct IpaUserFormatted {
        username: String,
        ssh_keys: String,
    }

    let mut formated_ipa_users: Vec<IpaUserFormatted> = vec![];
    for user in ipa_users {
        let mut ssh_keys: String = "".to_string();
        // May be unsafe if user without sshkey is added to booking, reminder to test before PR
        for ssh_key in user.ipasshpubkey.unwrap() {
            ssh_keys.push_str(format!("{} ", &ssh_key).as_str());
        }

        formated_ipa_users.push(IpaUserFormatted {
            username: user.uid,
            ssh_keys,
        });
    }

    let mut formatted_interfaces: Vec<String> = vec![];
    for interface in interfaces {
        formatted_interfaces.push(interface.name);
    }

    let formatted_mailboxes: MailboxPair = MailboxPair {
        preimage_waiter: preimage_endpoint.to_url(),
        imaging_waiter: postimage_endpoint.to_url(),
    };

    let mut template_context = tera::Context::new();

    template_context.insert("os_name", &iso_directory_name);
    template_context.insert("ipa_users", &formated_ipa_users);
    template_context.insert("vlan_configs", &vlan_configs);
    template_context.insert("interfaces", &formatted_interfaces);
    template_context.insert("mailbox_endpoint", &formatted_mailboxes);

    render_template("generic/kickstart.j2", &template_context)
}
