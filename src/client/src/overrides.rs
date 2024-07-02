use super::{areyousure, select_aggregate, select_host, select_instance, select_lifecyclestate};
use crate::mgmt_workflows::BootToDev;
use crate::remote::{Select, Server, Text};
use common::prelude::{
    anyhow,
    config::{settings, Situation},
    tracing,
};
use models::{
    allocation::Allocation,
    dal::{new_client, AsEasyTransaction, ExistingRow, FKey, ID},
    dashboard::{Aggregate, LifeCycleState},
    inventory::{BootTo, Host},
};
use notifications::{
    email::{send_to_admins_email, send_to_admins_gchat},
    send_test_email,
};
use std::io::Write;
use std::str::FromStr;
use strum::IntoEnumIterator;
use strum_macros::{Display, EnumIter, EnumString};
use tascii::prelude::Runtime;
use workflows::{
    deploy_booking::{
        deploy_host::DeployHost,
        notify::Notify,
        set_host_power_state::{PowerState, SetPower},
    },
    entry::DISPATCH,
    resource_management::{allocator, mailbox::Mailbox},
};

#[derive(Display, Clone, EnumString, EnumIter, Debug)]
pub enum Overrides {
    #[strum(serialize = "Override Aggregate")]
    Aggregate,
    #[strum(serialize = "Force Release Aggregate")]
    ReleaseAggregate,
    #[strum(serialize = "Reset Aggregate")]
    ResetAggregate,
    #[strum(serialize = "Boot Host")]
    BootHost,
    #[strum(serialize = "Redeploy Host")]
    Redeploy,
    #[strum(serialize = "Send Notification")]
    SendNotification,
    #[strum(serialize = "Send Email to Admins")]
    SendAdminEmail,
    #[strum(serialize = "Test Email Template")]
    TestEmailTemplate,
    #[strum(serialize = "Send Google Chat Notification")]
    SendAdminChat,
    #[strum(serialize = "Override Host Power State")]
    SetHostPowerState,
    #[strum(serialize = "Override Endpoint Hook")]
    EndpointHook,
}

pub async fn overrides(session: &Server, tascii_rt: &'static Runtime) -> Result<(), anyhow::Error> {
    let override_choice = Select::new("Select an override to apply:", Overrides::iter().collect())
        .prompt(session)
        .unwrap();

    match override_choice {
        Overrides::EndpointHook => handle_endpoint_hook(session).await,
        Overrides::SetHostPowerState => handle_set_host_power_state(session, tascii_rt).await,
        Overrides::SendAdminEmail => handle_send_admin_email(session).await,
        Overrides::TestEmailTemplate => handle_test_email_template(session).await,
        Overrides::SendAdminChat => handle_send_admin_chat(session).await,
        Overrides::Redeploy => handle_redeploy(session, tascii_rt).await,
        Overrides::Aggregate => handle_aggregate(session).await,
        Overrides::ResetAggregate => handle_reset_aggregate(session).await,
        Overrides::ReleaseAggregate => handle_release_aggregate(session).await,
        Overrides::BootHost => handle_boot_host(session, tascii_rt).await,
        Overrides::SendNotification => handle_send_notification(session, tascii_rt).await,
    }
}

async fn handle_endpoint_hook(session: &Server) -> Result<(), anyhow::Error> {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let lcs = select_lifecyclestate(session).unwrap();
    let agg = select_aggregate(session, lcs, &mut transaction)
        .await
        .unwrap();
    let inst = select_instance(session, agg, &mut transaction)
        .await
        .unwrap();
    let hook = Select::new(
        "enter hook name: ",
        Mailbox::live_hooks(inst).await.unwrap(),
    )
    .prompt(session)
    .unwrap();
    let hook = Mailbox::get_endpoint_hook(inst, &hook).await.unwrap();
    let msg = Text::new("Enter message for hook: ")
        .prompt(session)
        .unwrap();

    Mailbox::push(
        common::prelude::axum::extract::Path((hook.for_instance, hook.unique)),
        msg,
    )
    .await;
    println!("Pushed to the hook an override");

    transaction.commit().await?;
    Ok(())
}

async fn handle_set_host_power_state(
    session: &Server,
    tascii_rt: &'static Runtime,
) -> Result<(), anyhow::Error> {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let host = select_host(session, &mut transaction).await.unwrap();
    let pstate = Select::new(
        "select a desired power state: ",
        vec![PowerState::On, PowerState::Off, PowerState::Reset],
    )
    .prompt(session)
    .unwrap();
    areyousure(session)?;
    let id = tascii_rt.enroll(SetPower { host, pstate }.into());
    tascii_rt.set_target(id);

    transaction.commit().await?;
    Ok(())
}

async fn handle_send_admin_email(session: &Server) -> Result<(), anyhow::Error> {
    let email = Text::new("Message: ").prompt(session).unwrap();
    send_to_admins_email(email).await;
    Ok(())
}

async fn handle_test_email_template(session: &Server) -> Result<(), anyhow::Error> {
    let status = Select::new(
        "Select a Status: ",
        vec![
            Situation::BookingCreated,
            Situation::BookingExpiring,
            Situation::BookingExpired,
            Situation::RequestBookingExtension
        ],
    )
    .prompt(session)
    .unwrap();
    let project_type = Select::new(
        "Select a Project: ",
        settings()
            .projects
            .keys()
            .map(String::as_str)
            .collect::<Vec<&str>>(),
    )
    .prompt(session)
    .unwrap();
    let dest_user = Text::new("Destination Username: ").prompt(session).unwrap();

    let result = send_test_email(
        status,
        project_type.to_owned(),
        dest_user.clone(),
        Some(vec![dest_user.clone()]),
    )
    .await;

    match result {
        Ok(_) => tracing::info!("Successfully sent email to {:?}", &dest_user),
        Err(e) => tracing::error!("Failed to send email to {:?}: {:?}", &dest_user, e),
    }
    Ok(())
}

async fn handle_send_admin_chat(session: &Server) -> Result<(), anyhow::Error> {
    let gchat = Text::new("Message: ").prompt(session).unwrap();
    send_to_admins_gchat(gchat).await;
    Ok(())
}

async fn handle_redeploy(
    mut session: &Server,
    tascii_rt: &'static Runtime,
) -> Result<(), anyhow::Error> {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let agg = select_aggregate(session, LifeCycleState::New, &mut transaction)
        .await
        .unwrap();
    let inst = select_instance(session, agg, &mut transaction)
        .await
        .unwrap();
    let host_id = inst.get(&mut transaction).await.unwrap().linked_host;

    let host_id = if let Some(id) = host_id {
        id
    } else {
        let _ = writeln!(session, "No linked host, so can't simply restart");
        panic!()
    };

    let lifecycle_state = agg.get(&mut transaction).await.unwrap().into_inner().state;

    if let LifeCycleState::New = lifecycle_state {
        let _ = writeln!(
            session,
            "Cannot rerun host deployment while aggregate is still provisioning!"
        );
        return Ok(());
    }

    let allocation =
        Allocation::find_for_aggregate_and_host(&mut transaction, agg, host_id, false).await?;

    if allocation.is_empty() {
        let _ = writeln!(
            session,
            "No active allocation for {:?} and {:?} found! Cannot rerun.",
            &host_id, &agg
        );
        return Ok(());
    }

    if allocation.len() > 1 {
        let _ = writeln!(
            session,
            "Multiple active allocations for {:?} and {:?} found! Refusing to rerun.",
            &host_id, &agg
        );
        return Ok(());
    }
    areyousure(session)?;

    let task = DeployHost {
        host_id,
        aggregate_id: agg,
        using_instance: inst,
    };

    let id = tascii_rt.enroll(task.into());
    tascii_rt.set_target(id);

    format!("Reran host deploy as task id {id}");

    transaction.commit().await?;
    Ok(())
}

async fn handle_aggregate(mut session: &Server) -> Result<(), anyhow::Error> {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let agg_id = Text::new("aggregate to change state of:")
        .prompt(session)
        .unwrap();
    let agg_id = FKey::from_id(ID::from_str(agg_id.as_str()).unwrap());
    let mut agg: ExistingRow<Aggregate> = agg_id.get(&mut transaction).await.unwrap();

    let _ = writeln!(session, "Current state of agg is {:?}", agg.state);

    let new_state = Select::new(
        "Select a new state for the aggregate:",
        vec![
            LifeCycleState::New,
            LifeCycleState::Active,
            LifeCycleState::Done,
        ],
    )
    .prompt(session)
    .unwrap();

    agg.state = new_state;
    agg.update(&mut transaction).await.unwrap();

    let _ = writeln!(
        session,
        "State updated to {new_state} on aggregate {agg_id:?}"
    );

    transaction.commit().await?;
    Ok(())
}

async fn handle_reset_aggregate(mut session: &Server) -> Result<(), anyhow::Error> {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let agg_id = Text::new("aggregate to forcefully release and rerun:")
        .prompt(session)
        .unwrap();
    let agg_id = FKey::from_id(ID::from_str(agg_id.as_str()).unwrap());
    let mut agg: ExistingRow<Aggregate> = agg_id.get(&mut transaction).await.unwrap();

    areyousure(session)?;

    agg.state = LifeCycleState::New;
    agg.update(&mut transaction).await.unwrap();

    allocator::Allocator::instance()
        .deallocate_aggregate(&mut transaction, agg_id)
        .await
        .expect("couldn't dealloc agg");

    let _ = writeln!(
        session,
        "Released aggregate, deallocated hosts, now rerunning it..."
    );

    DISPATCH
        .get()
        .unwrap()
        .send(workflows::entry::Action::DeployBooking { agg_id: agg.id })
        .unwrap();

    let _ = writeln!(session, "Done starting rerun");

    transaction.commit().await?;
    Ok(())
}

async fn handle_release_aggregate(mut session: &Server) -> Result<(), anyhow::Error> {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let agg_id = Text::new("aggregate to forcefully release:")
        .prompt(session)
        .unwrap();
    let agg_id = FKey::from_id(ID::from_str(agg_id.as_str()).unwrap());
    let mut agg: ExistingRow<Aggregate> = agg_id.get(&mut transaction).await.unwrap();

    areyousure(session)?;

    agg.state = LifeCycleState::Done;
    agg.update(&mut transaction).await.unwrap();

    allocator::Allocator::instance()
        .deallocate_aggregate(&mut transaction, agg_id)
        .await
        .expect("couldn't dealloc agg");

    let _ = writeln!(session, "Released aggregate, deallocated hosts");

    transaction.commit().await?;
    Ok(())
}

async fn handle_boot_host(
    session: &Server,
    tascii_rt: &'static Runtime,
) -> Result<(), anyhow::Error> {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let hostname = Text::new("hostname:").prompt(session).unwrap();
    let bootdev = Select::new("select boot device:", vec![BootTo::Disk, BootTo::Network])
        .prompt(session)
        .unwrap();
    let resource = Host::get_by_name(&mut transaction, hostname)
        .await
        .expect("no host found by that hostname")
        .into_inner();

    let task = BootToDev {
        host: resource.id,
        bootdev,
    };

    let id = tascii_rt.enroll(task.into());
    tascii_rt.set_target(id);

    format!("Enrolled boot dev task as id {id:?}");

    transaction.commit().await?;
    Ok(())
}

async fn handle_send_notification(
    mut session: &Server,
    tascii_rt: &'static Runtime,
) -> Result<(), anyhow::Error> {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let lcs = Select::new(
        "choose aggregate state from which to select an aggregate to notify about: ",
        vec![
            LifeCycleState::New,
            LifeCycleState::Active,
            LifeCycleState::Done,
        ],
    )
    .prompt(session)
    .unwrap();
    let agg = select_aggregate(session, lcs, &mut transaction).await?;
    let situation = Select::new(
        "select situation to send notification for: ",
        vec![
            Situation::BookingCreated,
            Situation::BookingExpired,
            Situation::BookingExpiring,
            Situation::VPNAccessAdded,
            Situation::VPNAccessRemoved,
            Situation::PasswordResetRequested,
        ],
    )
    .prompt(session)?;

    let task = Notify {
        aggregate: agg,
        situation,
        extra_context: vec![],
    };

    let id = tascii_rt.enroll(task.into());
    tascii_rt.set_target(id);

    let _ = writeln!(session, "Started notify task");

    transaction.commit().await?;
    Ok(())
}
