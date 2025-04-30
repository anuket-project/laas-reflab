use crate::remote::{Select, Server, Text};
use crate::{areyousure, get_lab, select_host};
use common::prelude::anyhow;
use dal::{get_db_pool, new_client, AsEasyTransaction, DBTable, EasyTransaction, FKey};

use models::{
    allocator::{Allocation, ResourceHandle},
    dashboard::{Aggregate, BookingMetadata, Instance, LifeCycleState, Network, ProvisionLogEvent},
    inventory::{Host, Vlan},
};
use std::io::Write;
use strum::IntoEnumIterator;
use strum_macros::{Display, EnumIter, EnumString};
use uuid::Uuid;
use workflows::{
    deploy_booking::set_host_power_state::{get_host_power_state, HostConfig, PowerStateError},
    resource_management::allocator,
};

#[derive(Clone, Debug, Display, EnumString, EnumIter)]
pub enum Queries {
    #[strum(serialize = "Summarize Aggregate")]
    Aggregate,
    #[strum(serialize = "Query Host Config")]
    Config,
    #[strum(serialize = "Summarize Current Bookings")]
    Summarize,
    #[strum(serialize = "Display IPMI Credentials")]
    IpmiCredentials,
    #[strum(serialize = "List Free Vlans")]
    FreeVlans,
    #[strum(serialize = "List Free Hosts")]
    FreeHosts,
    #[strum(serialize = "Query Host Power State")]
    HostPowerState,
    #[strum(serialize = "Find Leaked Bookings")]
    LeakedBooking,
    #[strum(serialize = "Query BMC/IPMI VLAN for Host")]
    BMCVlan,
}

pub async fn query(session: &Server) -> Result<(), anyhow::Error> {
    let query_choice =
        Select::new("What would you like to do?:", Queries::iter().collect()).prompt(session)?;

    match query_choice {
        Queries::HostPowerState => handle_host_power_state_query(session).await,
        Queries::FreeVlans => handle_free_vlans_query(session).await,
        Queries::FreeHosts => handle_free_hosts_query(session).await,
        Queries::IpmiCredentials => handle_ipmi_credentials_query(session).await,
        Queries::Summarize => handle_summarize_query(session).await,
        Queries::Aggregate => handle_aggregate_query(session).await,
        Queries::Config => handle_config_query(session).await,
        Queries::LeakedBooking => handle_leaked_query(session).await,
        Queries::BMCVlan => handle_bmc_vlan_query(session).await,
    }
}

async fn handle_host_power_state_query(mut session: &Server) -> Result<(), anyhow::Error> {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let host = select_host(session, &mut transaction).await.unwrap();
    let host = host.get(&mut transaction).await.unwrap();

    let ipmi_fqdn = &host.ipmi_fqdn;

    match get_host_power_state(&HostConfig::try_from(host.clone().into_inner()).unwrap()).await {
        Ok(power_state) => {
            let msg = format!(
                "Host {} is currently in power state: {:?}\n",
                ipmi_fqdn, power_state
            );
            writeln!(session, "{}", msg).expect("Failed to write to session");
        }
        Err(e) => {
            let error_msg = match e {
                PowerStateError::CommandNonZeroExitStatus(code, err_msg) => {
                    format!(
                        "Failed to get power state for host {}. Exit code: {}. Error: {}\n",
                        ipmi_fqdn, code, err_msg
                    )
                }
                PowerStateError::CommandExecutionFailed(err_msg) => {
                    format!(
                        "Command execution failed for host {}: {}\n",
                        ipmi_fqdn, err_msg
                    )
                }
                PowerStateError::UnknownPowerState(err_msg) => {
                    format!("Unknown power state for host {}: {}\n", ipmi_fqdn, err_msg)
                }
                PowerStateError::InvalidInputParameter(param) => {
                    format!(
                        "Invalid input parameter for host {}: {}\n",
                        ipmi_fqdn, param
                    )
                }
                PowerStateError::Utf8Error(err) => {
                    format!("UTF-8 encoding error for host {}: {}\n", ipmi_fqdn, err)
                }
                PowerStateError::TimeoutReached => {
                    format!(
                        "Timeout reached while getting power state for host {}\n",
                        ipmi_fqdn
                    )
                }
                PowerStateError::SetUnknown => {
                    format!(
                        "Attempted to set an unknown power state for host {}\n",
                        ipmi_fqdn
                    )
                }
                PowerStateError::HostUnreachable(host) => {
                    format!("Host {} is unreachable.", host)
                }
            };
            writeln!(session, "{}", error_msg).expect("Failed to write to session");
        }
    }

    transaction.commit().await?;
    Ok(())
}

async fn handle_free_vlans_query(mut session: &Server) -> Result<(), anyhow::Error> {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let lab = get_lab(session, &mut transaction).await?;

    let mut vlans = allocator::Allocator::instance()
        .get_free_vlans(&mut transaction, lab)
        .await
        .unwrap();

    vlans.sort_by_key(|v| v.0.vlan_id);

    writeln!(session, "Free vlans:")?;
    for (vlan, _) in vlans {
        let vid = vlan.vlan_id;
        let id = vlan.id.into_id();
        let pc = vlan.public_config.clone();
        let public = if pc.is_some() { "public" } else { "private" };

        writeln!(session, "- {id} | vlan id {vid} | {public}")?;
    }
    writeln!(session, "=====")?;

    transaction.commit().await?;
    Ok(())
}

async fn handle_free_hosts_query(mut session: &Server) -> Result<(), anyhow::Error> {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let lab = get_lab(session, &mut transaction).await?;

    let hosts = allocator::Allocator::instance()
        .get_free_hosts(&mut transaction, lab)
        .await
        .unwrap();

    writeln!(session, "Free hosts:")?;
    for (host, _) in hosts {
        let hs = summarize_host(&mut transaction, host.id).await;

        writeln!(session, "- {hs}")?;
    }
    writeln!(session, "=====")?;

    transaction.commit().await?;
    Ok(())
}

async fn handle_ipmi_credentials_query(mut session: &Server) -> Result<(), anyhow::Error> {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let (host, _handle) = get_host_by_hostname(session, &mut transaction).await;

    writeln!(
        session,
        "IPMI (FQDN, User, Pass, MAC):\n{}\n{}\n{}\n{}",
        host.ipmi_fqdn, host.ipmi_user, host.ipmi_pass, host.ipmi_mac
    )?;

    transaction.commit().await?;
    Ok(())
}

async fn handle_summarize_query(session: &Server) -> Result<(), anyhow::Error> {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let state = Select::new(
        "Get bookings in state:",
        vec![
            LifeCycleState::New,
            LifeCycleState::Active,
            LifeCycleState::Done,
        ],
    )
    .prompt(session)
    .unwrap();

    let aggregates = Aggregate::select()
        .where_field("lifecycle_state")
        .equals(state)
        .run(&mut transaction)
        .await
        .unwrap();

    for agg in aggregates {
        summarize_aggregate(session, &mut transaction, agg.id).await?;
    }

    transaction.commit().await?;
    Ok(())
}

async fn handle_aggregate_query(mut session: &Server) -> Result<(), anyhow::Error> {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let hostname = Text::new("hostname:").prompt(session).unwrap();
    let resource = Host::get_by_name(&mut transaction, hostname)
        .await
        .expect("no host found by that hostname")
        .into_inner();
    let handle = ResourceHandle::handle_for_host(&mut transaction, resource.id)
        .await
        .expect("host didn't have a resource handle");

    let current_allocations = Allocation::find(&mut transaction, handle.id, false)
        .await
        .unwrap();

    match current_allocations.as_slice() {
        [] => {
            writeln!(session, "Host is not currently a member of an allocation")?;
        }
        [one] => {
            let fa = one.for_aggregate;
            let a = one.id;

            writeln!(
                session,
                "Host is within allocation {a:?}, which is part of aggregate {fa:?}"
            )?;

            if let Some(aid) = fa {
                summarize_aggregate(session, &mut transaction, aid).await?;
            }
        }
        more => {
            unreachable!("Host was a member of multiple allocations, they are {more:?}, which is a DB integrity issue!")
        }
    }

    transaction.commit().await?;
    Ok(())
}

async fn handle_config_query(mut session: &Server) -> Result<(), anyhow::Error> {
    let mut client = new_client().await.expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let hostname = Text::new("hostname:").prompt(session).unwrap();
    let resource = Host::get_by_name(&mut transaction, hostname)
        .await
        .expect("no host found by that hostname")
        .into_inner();
    let handle = ResourceHandle::handle_for_host(&mut transaction, resource.id)
        .await
        .expect("host didn't have a resource handle");

    let allocation = Allocation::find(&mut transaction, handle.id, false)
        .await
        .unwrap()
        .first()
        .unwrap()
        .clone();

    let agg = allocation
        .for_aggregate
        .unwrap()
        .get(&mut transaction)
        .await
        .unwrap()
        .into_inner();

    for inst in agg.instances(&mut transaction).await.unwrap() {
        let inst = inst.into_inner();
        if let Some(h) = inst.linked_host
            && h == resource.id
        {
            // found our host, now can look at config
            let conf = inst.config;
            let image = conf.image.get(&mut transaction).await.unwrap().into_inner();
            let hostname = conf.hostname.clone();
            writeln!(session, "Hostname {hostname}")?;
            writeln!(
                session,
                "Assigned image: {}, cobbler id {}, id {:?}",
                image.name, image.cobbler_name, image.id
            )?;
            let generated = workflows::deploy_booking::generate_cloud_config(
                conf.clone(),
                h,
                inst.id,
                agg.id,
                &mut transaction,
            )
            .await
            .unwrap();

            writeln!(session, "Primary CI file:")?;
            writeln!(session, "{generated}")?;
            writeln!(session, "=======")?;

            for cif in conf.cifile {
                let cif = cif.get(&mut transaction).await.unwrap().into_inner();
                writeln!(
                    session,
                    "Additional CI file {:?}, priority {}:",
                    cif.id, cif.priority
                )?;
                writeln!(session, "=== BEGIN CONFIG FILE ===")?;
                writeln!(session, "{}", cif.data)?;
                writeln!(session, "==== END CONFIG FILE ====")?;
            }
        }
    }

    transaction.commit().await?;
    Ok(())
}

async fn handle_leaked_query(mut session: &Server) -> Result<(), anyhow::Error> {
    let aggregate_tn = Aggregate::table_name();
    let allocation_tn = Allocation::table_name();
    let resource_handle_tn = ResourceHandle::table_name();

    let _ = writeln!(session, "Finding potentially leaked bookings. THIS IS FOR DIAGNOSTIC PURPOSES ONLY. A booking is NOT guarenteed to be leaked if it appears here. The dashboard is the only source of truth for booking end dates.");
    areyousure(session)?;

    let active_not_ended_query = format!(
        "
        select
            id as agg_id,
            (metadata ->> 'end') :: text as expected_end
        from
            {aggregate_tn}
        where
            (metadata ->> 'end') :: timestamp < NOW()
            and lifecycle_state = '\"Active\"'
        order by
            expected_end
    "
    );

    let mut client = new_client().await.expect("Expected to connect to db");
    let t = client
        .easy_transaction()
        .await
        .expect("Transaction creation error");

    let active_not_ended_rows = t
        .query(&active_not_ended_query, &[])
        .await
        .expect("Expected to query for active bookings that were not ended!");

    let _ = writeln!(session, "Aggregates with lifecycle_state = \"Active\" and end < NOW()\n-----------------------------");
    for row in active_not_ended_rows {
        let agg_id: Uuid = row.get("agg_id");
        let expected_end: String = row.get("expected_end");

        let _ = writeln!(
            session,
            "Aggregate ID: {agg_id}, Expected End: {expected_end}"
        );
    }

    let has_allocations_query = format!(
        "
        select
            aggs.id as agg_id,
            a.id as alloc_id,
            expected_end,
            rh.tracks_resource_type as resource_type
        from
            {allocation_tn} a
            join (
                select
                    id,
                    (metadata ->> 'end') :: text as expected_end
                from
                    {aggregate_tn}
                where
                    (metadata ->> 'end') :: timestamp < NOW()
            ) as aggs on a.for_aggregate = aggs.id
            join {resource_handle_tn} rh on a.for_resource = rh.id
        where
            ended is null
            and rh.tracks_resource_type != 'vpn'
        order by
            expected_end;
        "
    );

    let has_allocations_rows = t
        .query(&has_allocations_query, &[])
        .await
        .expect("Expected to query for bookings with allocations and end < NOW()");

    let _ = writeln!(
        session,
        "\nAggregates with live allocations and end < NOW()\n-----------------------------"
    );
    for row in has_allocations_rows {
        let agg_id: Uuid = row.get("agg_id");
        let expected_end: String = row.get("expected_end");
        let alloc_id: Uuid = row.get("alloc_id");
        let resource_type: String = row.get("resource_type");

        let _ = writeln!(session, "Aggregate ID: {agg_id}, Allocation ID: {alloc_id}, Resource Type: {resource_type}, Expected End: {expected_end}");
    }
    Ok(())
}

pub async fn handle_bmc_vlan_query(mut session: &Server) -> Result<(), anyhow::Error> {
    let pool = get_db_pool().await?;

    let host_name = Text::new("Enter the hostname (e.g., hpe1):")
        .prompt(session)
        .map_err(|e| anyhow::anyhow!("Failed to read input: {}", e))?;

    let rows = sqlx::query!(
        "SELECT hp.bmc_vlan_id, hp.mac, s.name as switch_name, sp.name AS switchport_name 
         FROM host_ports hp
         JOIN hosts h ON hp.on_host = h.id
         JOIN switchports sp ON hp.switchport = sp.id
         JOIN switches s ON sp.for_switch = s.id
         WHERE h.server_name = $1",
        host_name
    )
    .fetch_all(&pool)
    .await?;

    if rows.is_empty() {
        writeln!(session, "No host ports found for {}", host_name)?;
    } else {
        for row in rows {
            writeln!(
                session,
                "Host: {} | Switchport: {} | MAC: {} | BMC VLAN: {}",
                host_name,
                row.switchport_name,
                row.mac.map_or(String::from("Null"), |mac| mac.to_string()),
                row.bmc_vlan_id
                    .map_or(String::from("Null"), |id| id.to_string())
            )?;
        }
    }

    Ok(())
}

async fn summarize_aggregate(
    mut session: &Server,
    transaction: &mut EasyTransaction<'_>,
    agg_id: FKey<Aggregate>,
) -> anyhow::Result<()> {
    let agg = agg_id.get(transaction).await.unwrap().into_inner();

    let _allocations = Allocation::all_for_aggregate(transaction, agg.id)
        .await
        .unwrap();

    writeln!(session, "===== Aggregate by id {:?}", agg.id)?;

    let BookingMetadata {
        booking_id,
        owner,
        lab,
        purpose,
        project,
        start,
        end,
    } = agg.metadata.clone();

    writeln!(session, "Booking ID: {booking_id:?}")?;
    writeln!(session, "Purpose: {purpose:?}")?;
    writeln!(session, "Owned by: {owner:?}")?;
    writeln!(session, "Start: {start:?}")?;
    writeln!(session, "End: {end:?}")?;
    writeln!(session, "Lab: {lab:?}")?;
    writeln!(session, "Project: {project:?}")?;

    writeln!(session, "Collaborators:")?;
    for user in agg.users.iter() {
        writeln!(session, "- {user}")?;
    }

    writeln!(session, "Networks:")?;
    for (net, vlan) in agg
        .vlans
        .get(transaction)
        .await
        .unwrap()
        .into_inner()
        .networks
    {
        let net = net.get(transaction).await.unwrap().into_inner();
        let vlan = vlan.get(transaction).await.unwrap().into_inner();

        let Network {
            id: _n_id,
            name,
            public: _,
        } = net;
        let Vlan {
            id: _v_id,
            vlan_id,
            public_config: _,
        } = vlan;

        writeln!(session, "- {name} with assigned vlan {vlan_id}")?;
    }

    writeln!(session, "Resources:")?;
    for instance in agg.instances(transaction).await.unwrap() {
        let instance = instance.into_inner();

        let Instance {
            id: _,
            metadata: _,
            aggregate: _,
            within_template: _,
            config,
            network_data: _,
            linked_host,
        } = instance;

        let host = match linked_host {
            Some(h) => {
                let h = h.get(transaction).await.unwrap().into_inner();
                h.server_name.to_string()
            }
            None => {
                let inst_of = config.flavor.get(transaction).await.unwrap().name.clone();
                format!("<unassigned host of type {inst_of}>")
            }
        };

        let config = {
            let hn = config.hostname;
            let img = config.image.get(transaction).await.unwrap().into_inner();
            let img_name = img.name;
            let img_cname = img.cobbler_name;

            format!("{{ hostname {hn}, image {img_name} which in cobbler is {img_cname} }}")
        };

        writeln!(session, "- Host {host} with config {config}")?;
        writeln!(session, "  - Log events:")?;
        let mut events = ProvisionLogEvent::all_for_instance(transaction, instance.id)
            .await
            .unwrap_or(vec![]);
        events.sort_by_key(|e| e.time);
        for ev in events {
            let time = ev.time.to_rfc2822();
            let content = ev.prov_status.to_string();
            writeln!(session, "    - {time}: {content}")?;
        }
    }
    writeln!(session, "=========\n")?;
    Ok(())
}

async fn summarize_host(transaction: &mut EasyTransaction<'_>, host: FKey<Host>) -> String {
    let host = host.get(transaction).await.unwrap();
    host.server_name.to_string()
}

async fn get_host_by_hostname(
    session: &Server,
    transaction: &mut EasyTransaction<'_>,
) -> (Host, ResourceHandle) {
    let hostname = Text::new("hostname:").prompt(session).unwrap();
    let resource = Host::get_by_name(transaction, hostname)
        .await
        .expect("no host found by that hostname")
        .into_inner();
    let handle = ResourceHandle::handle_for_host(transaction, resource.id)
        .await
        .expect("host didn't have a resource handle");

    (resource, handle)
}
