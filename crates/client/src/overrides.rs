use super::{areyousure, select_aggregate, select_host, select_instance};

use crate::mgmt_workflows::BootToDev;
use crate::remote::{Select, Server, Text};

use common::prelude::{anyhow, config::Situation, tracing};
use dal::{new_client, AsEasyTransaction, ExistingRow, FKey, ID};

use models::{
    allocator::Allocation,
    dashboard::{Aggregate, LifeCycleState},
    inventory::{BootTo, Host},
};

use dal::NewRow;
use models::{
    allocator::{AllocationReason, ResourceHandle, ResourceHandleInner},
    dashboard::{AggregateConfiguration, BookingMetadata, NetworkAssignmentMap, Template},
    inventory::Lab,
};

use std::io::Write;
use std::str::FromStr;
use strum::IntoEnumIterator;
use strum_macros::{Display, EnumIter, EnumString};
use tascii::prelude::Runtime;
use tracing::info;
use workflows::resource_management::allocator::Allocator;
use workflows::{
    deploy_booking::{
        deploy_host::DeployHost,
        notify::Notify,
        set_host_power_state::{PowerState, SetPower},
    },
    entry::DISPATCH,
    resource_management::allocator,
};

#[derive(Display, Clone, EnumString, EnumIter, Debug)]
pub enum AggregateActions {
    #[strum(serialize = "Set Aggregate Lifecycle State")]
    AggregateLifecycleState,
    #[strum(serialize = "Force Release Aggregate")]
    ReleaseAggregate,
    #[strum(serialize = "ReallocateAggregate")]
    ReallocateAggregate,
    #[strum(serialize = "Boot Host")]
    BootHost,
    #[strum(serialize = "Redeploy Host")]
    Redeploy,
    #[strum(serialize = "Send Notification for Aggregate")]
    SendNotificationForAggregate,
    #[strum(serialize = "Mark Host Not Working")]
    MarkHostNotWorking,
    #[strum(serialize = "Override Host Power State")]
    SetHostPowerState,
}

pub async fn overrides(session: &Server, tascii_rt: &'static Runtime) -> Result<(), anyhow::Error> {
    let override_choice = Select::new(
        "Select an override to apply:",
        AggregateActions::iter().collect(),
    )
    .prompt(session)
    .unwrap();

    match override_choice {
        AggregateActions::SetHostPowerState => {
            handle_set_host_power_state(session, tascii_rt).await
        }
        AggregateActions::Redeploy => handle_redeploy(session, tascii_rt).await,
        AggregateActions::AggregateLifecycleState => handle_aggregate_state_override(session).await,
        AggregateActions::ReallocateAggregate => handle_reallocate_aggregate(session).await,
        AggregateActions::ReleaseAggregate => handle_release_aggregate(session).await,
        AggregateActions::BootHost => handle_boot_host(session, tascii_rt).await,
        AggregateActions::SendNotificationForAggregate => {
            handle_send_notification(session, tascii_rt).await
        }
        AggregateActions::MarkHostNotWorking => handle_mark_host_not_working(session).await,
    }
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

/// Runs the DeployHost task for a host within an Active aggregate.
/// Cannot be run if the aggregate is still completing the initial provision or if the booking has ended.
async fn handle_redeploy(
    mut session: &Server,
    tascii_rt: &'static Runtime,
) -> Result<(), anyhow::Error> {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let allowed_lifecycle = LifeCycleState::Active;

    let agg = select_aggregate(session, allowed_lifecycle, &mut transaction)
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

    if lifecycle_state != allowed_lifecycle {
        let _ = writeln!(
            session,
            "Cannot rerun host deployment while aggregate is still provisioning or if the booking has ended!"
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
        distribution: None,
    };

    let id = tascii_rt.enroll(task.into());
    tascii_rt.set_target(id);

    info!("Reran host deploy as task id {id}");

    transaction.commit().await?;
    Ok(())
}

async fn handle_aggregate_state_override(mut session: &Server) -> Result<(), anyhow::Error> {
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

async fn handle_reallocate_aggregate(mut session: &Server) -> Result<(), anyhow::Error> {
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

    info!("Enrolled boot dev task as id {id:?}");

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

async fn handle_mark_host_not_working(session: &Server) -> Result<(), anyhow::Error> {
    let hostname = Text::new("Enter the hostname to mark as not working:")
        .prompt(session)
        .unwrap();
    let reason = Text::new("Enter reason:").prompt(session).unwrap();

    mark_host_not_working(hostname, reason).await
}

pub async fn mark_host_not_working(hostname: String, reason: String) -> Result<(), anyhow::Error> {
    let mut client = new_client().await?;
    let mut transaction = client.easy_transaction().await?;

    let allocator = Allocator::instance();

    let host = Host::get_by_name(&mut transaction, hostname.clone()).await?;

    let handle = ResourceHandle::handle_for_host(&mut transaction, host.id).await?;

    let lab = Lab::default().id;

    let agg = Aggregate {
        id: FKey::new_id_dangling(),
        deleted: false,
        users: vec![],
        vlans: NewRow::new(NetworkAssignmentMap::empty())
            .insert(&mut transaction)
            .await?,
        template: NewRow::new(Template {
            id: FKey::new_id_dangling(),
            name: format!("maintenance-{}", hostname),
            deleted: false,
            description: reason.clone(),
            owner: None,
            public: false,
            networks: vec![],
            hosts: vec![],
            lab,
        })
        .insert(&mut transaction)
        .await?,
        metadata: BookingMetadata {
            booking_id: None,
            owner: None,
            lab: None,
            purpose: Some("Maintenance".to_string()),
            project: None,
            details: Some(reason.clone()),
            start: None,
            end: None,
        },
        state: LifeCycleState::Active,
        configuration: AggregateConfiguration {
            ipmi_username: String::new(),
            ipmi_password: String::new(),
        },
        lab,
    };

    let agg_id = NewRow::new(agg.clone()).insert(&mut transaction).await?;

    if let ResourceHandleInner::Host(h) = handle.tracks {
        allocator
            .allocate_specific_host(
                &mut transaction,
                h,
                agg_id,
                AllocationReason::ForMaintenance,
            )
            .await?;
    } else {
        anyhow::bail!("ResourceHandle did not point to a Host");
    }

    transaction.commit().await?;
    tracing::info!(
        "Host {} marked not working → Aggregate {:?}",
        hostname,
        agg_id
    );

    Ok(())
}
