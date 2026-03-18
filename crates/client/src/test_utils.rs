use crate::remote::{Select, Server, Text};
use crate::{get_lab, switch_test};
use common::prelude::anyhow;
use common::prelude::rand::random;
use dal::{AsEasyTransaction, FKey, ID, new_client};
use metrics::{BookingExpiredMetric, BookingMetric, MetricHandler, ProvisionMetric};
use models::dashboard::types::Distro;
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
    #[strum(serialize = "Test NXOS Switch")]
    TestSwitch,
    #[strum(serialize = "Test NXOS VLAN Configuration")]
    TestVlanConfig,
    #[strum(serialize = "Send a Mock Provision Metric")]
    TestSendProvisionMetric,
    #[strum(serialize = "Send a Mock Booking Metric")]
    TestSendBookingMetric,
    #[strum(serialize = "Send a Mock Booking Expired Metric")]
    TestSendBookingExpiredMetric,
}

pub async fn test_utils(session: &Server) -> Result<(), anyhow::Error> {
    let choice =
        Select::new("What would you like to do?:", TestUtils::iter().collect()).prompt(session)?;

    match choice {
        TestUtils::RenderAutoinstallTemplate => {
            handle_test_render_autoinstall_and_ks_template(session).await
        }
        TestUtils::TestSwitch => switch_test::test_switch(session).await,
        TestUtils::TestVlanConfig => switch_test::test_vlan_configuration(session).await,
        TestUtils::TestSendProvisionMetric => test_send_provision_metric(session).await,
        TestUtils::TestSendBookingMetric => test_send_booking_metric(session).await,
        TestUtils::TestSendBookingExpiredMetric => test_send_booking_expired_metric(session).await,
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

    let postprovision_endpoint: Endpoint = Endpoint {
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
        postprovision_endpoint,
        None,
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
        None,
    )?;

    writeln!(session, "{rendered_template}")?;

    Ok(())
}

async fn test_send_provision_metric(mut session: &Server) -> Result<(), anyhow::Error> {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let provision_metric = ProvisionMetric {
        hostname: Some("CLI Test Host".to_string()),
        success: true,
        retries: 1,
        provisioning_time_seconds: 30 * 60,
        owner: "Test Owner".to_string(),
        lab: get_lab(session, &mut transaction)
            .await?
            .get(&mut transaction)
            .await?
            .name
            .clone(),
        project: Some("Test Project".to_string()),
        distro: Select::new(
            "What distro would you like to use:",
            Distro::iter().collect(),
        )
        .prompt(session)
        .unwrap()
        .to_string(),
        image: Text::new("What image would you like to use:")
            .prompt(session)
            .unwrap()
            .to_string(),
        mock: true,
        ..Default::default()
    };

    transaction.commit().await.unwrap();

    if let Err(e) = MetricHandler::send(provision_metric) {
        writeln!(session, "Failed to send provision metric: {:?}", e)?;
    } else {
        writeln!(session, "Provision metric sent successfully")?;
    }

    Ok(())
}

async fn test_send_booking_metric(mut session: &Server) -> Result<(), anyhow::Error> {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let booking_metric = BookingMetric {
        booking_id: random(),
        booking_length_days: Select::new("Select the length of the booking", (0..=21).collect())
            .prompt(session)
            .unwrap(),
        num_hosts: Select::new("Select the number of hosts", (0..=6).collect())
            .prompt(session)
            .unwrap(),
        num_collaborators: Select::new("Select the number of collaborators", (0..=5).collect())
            .prompt(session)
            .unwrap(),
        owner: "Test Owner".to_string(),
        lab: get_lab(session, &mut transaction)
            .await?
            .get(&mut transaction)
            .await?
            .name
            .clone(),
        project: "Test Project".to_string(),
        purpose: Some("Test Purpose".to_string()),
        details: Some("Test Details".to_string()),
        mock: true,
        ..Default::default()
    };

    transaction.commit().await.unwrap();

    if let Err(e) = MetricHandler::send(booking_metric) {
        writeln!(session, "Failed to send provision metric: {:?}", e)?;
    } else {
        writeln!(session, "Provision metric sent successfully")?;
    }

    Ok(())
}

async fn test_send_booking_expired_metric(mut session: &Server) -> Result<(), anyhow::Error> {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let booking_expired_metric = BookingExpiredMetric {
        owner: "Test Owner".to_string(),
        booking_id: random(),
        total_booking_length_days: Select::new(
            "Select the booking total length",
            (1..=9999).collect(),
        )
        .prompt(session)
        .unwrap(),
        project: "Test Project".to_string(),
        lab: get_lab(session, &mut transaction)
            .await?
            .get(&mut transaction)
            .await?
            .name
            .clone(),
        mock: true,
        ..Default::default()
    };

    transaction.commit().await.unwrap();

    if let Err(e) = MetricHandler::send(booking_expired_metric) {
        writeln!(session, "Failed to send provision metric: {:?}", e)?;
    } else {
        writeln!(session, "Provision metric sent successfully")?;
    }

    Ok(())
}
