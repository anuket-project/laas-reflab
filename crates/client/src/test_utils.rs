use crate::remote::{Select, Server};
use common::prelude::anyhow;
use dal::{FKey, ID};

use models::inventory::HostPort;

use std::io::Write;
use strum::IntoEnumIterator;
use strum_macros::{Display, EnumIter, EnumString};
use users::ipa;
use workflows::configure_networking::vlan_connection::NetworkManagerVlanConnection;
use workflows::resource_management::mailbox::Endpoint;
use workflows::{render_autoinstall_template, render_kickstart_template};

#[derive(Clone, Debug, Display, EnumString, EnumIter)]
pub enum TestUtils {
    #[strum(serialize = "Render Autoinstall / Kickstart Jinja Template")]
    RenderAutoinstallTemplate,
}

pub async fn test_utils(session: &Server) -> Result<(), anyhow::Error> {
    let choice =
        Select::new("What would you like to do?:", TestUtils::iter().collect()).prompt(session)?;

    match choice {
        TestUtils::RenderAutoinstallTemplate => {
            handle_test_render_autoinstall_and_ks_template(session).await
        }
    }
}

async fn handle_test_render_autoinstall_and_ks_template(
    mut session: &Server,
) -> Result<(), anyhow::Error> {
    let ipa_users: Vec<ipa::User> = vec![
        ipa::User {
            uid: "has-an-ssh-key".to_string(),
            givenname: "test-gn".to_string(),
            sn: "test-sn".to_string(),
            cn: None,
            homedirectory: None,
            gidnumber: None,
            displayname: None,
            loginshell: None,
            mail: "testmail".to_string(),
            userpassword: None,
            random: None,
            uidnumber: None,
            ou: "test-ou".to_string(),
            title: None,
            ipasshpubkey: Some(vec!["abcdef ssh-rsa testemail@mail.com".to_string()]),
            ipauserauthtype: None,
            userclass: None,
            usercertificate: None,
        },
        ipa::User {
            uid: "empty-ssh-key".to_string(),
            givenname: "test-gn".to_string(),
            sn: "test-sn".to_string(),
            cn: None,
            homedirectory: None,
            gidnumber: None,
            displayname: None,
            loginshell: None,
            mail: "testmail".to_string(),
            userpassword: None,
            random: None,
            uidnumber: None,
            ou: "test-ou".to_string(),
            title: None,
            ipasshpubkey: Some(vec![]),
            ipauserauthtype: None,
            userclass: None,
            usercertificate: None,
        },
        ipa::User {
            uid: "none-ssh-key".to_string(),
            givenname: "test-gn".to_string(),
            sn: "test-sn".to_string(),
            cn: None,
            homedirectory: None,
            gidnumber: None,
            displayname: None,
            loginshell: None,
            mail: "testmail".to_string(),
            userpassword: None,
            random: None,
            uidnumber: None,
            ou: "test-ou".to_string(),
            title: None,
            ipasshpubkey: None,
            ipauserauthtype: None,
            userclass: None,
            usercertificate: None,
        },
    ];
    let preimage_endpoint: Endpoint = Endpoint {
        for_instance: FKey::new_id_dangling(),
        unique: ID::new(),
    };
    let postimage_endpoint: Endpoint = Endpoint {
        for_instance: FKey::new_id_dangling(),
        unique: ID::new(),
    };
    let hostname: String = "CLI Test Host".to_string();
    let ports: Vec<HostPort> = vec![];
    let nm_connections: Vec<NetworkManagerVlanConnection> = vec![NetworkManagerVlanConnection {
        device_name: "ens-test".to_string(),
        network_name: "test-network".to_string(),
        vlan_id: 200,
        tagged: true,
        connection_number: 1,
    }];

    writeln!(
        session,
        "\nRendering autoinstall template\n-----------------------\n"
    )?;

    let rendered_template = render_autoinstall_template(
        ipa_users.clone(),
        preimage_endpoint,
        postimage_endpoint,
        hostname,
        ports.clone(),
        nm_connections.clone(),
    )?;

    writeln!(session, "{rendered_template}")?;

    writeln!(
        session,
        "\nRendering kickstart template\n-----------------------\n"
    )?;
    let vlan_configs = nm_connections
        .iter()
        .map(|nm| nm.render_kickstart_network_config())
        .collect();
    let hostname = "test-host".to_string();

    let rendered_template = render_kickstart_template(
        config::settings().pxe.address.clone(),
        "/render-test".to_string(),
        ipa_users,
        ports,
        vlan_configs,
        hostname,
        preimage_endpoint,
        postimage_endpoint,
    )?;

    writeln!(session, "{rendered_template}")?;

    Ok(())
}
