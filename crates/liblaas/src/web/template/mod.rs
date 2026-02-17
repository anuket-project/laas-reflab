//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use std::collections::HashMap;

use common::prelude::{itertools::Itertools, *};

use aide::axum::{
    routing::{delete, get, post},
    ApiRouter,
};
use axum::{extract::Path, Json};
use dal::{web::*, *};

use models::{
    dashboard::{
        self, BondGroupConfig, HostConfig, Network, NetworkBlob, Template, VlanConnectionConfig,
    },
    inventory::{DataUnit, DataValue, Lab},
};

use axum::http::StatusCode;
use tracing::info;

use crate::web::api::{self};

use super::{
    api::{BondgroupBlob, ConnectionBlob, HostConfigBlob, InterfaceBlob, TemplateBlob},
    AppState, WebError,
};

pub async fn list_templates(
    Path((request_origin, username)): Path<(String, String)>,
) -> Result<Json<Vec<TemplateBlob>>, WebError> {
    // Lists all templates available to a given user
    tracing::info!("API call to list_templates()");

    // Connect
    let mut client = new_client().await.log_db_client_error()?;
    let mut transaction = client.easy_transaction().await.log_db_client_error()?;
    let t = &mut transaction;

    let mut templates: HashMap<ID, Template> = Template::get_public(t)
        .await
        .log_server_error("unable to get public templates", true)?
        .into_iter()
        .map(|t| (t.id.into_id(), t))
        .collect();
    tracing::debug!("public templates: {templates:?}");
    let user_templates = Template::owned_by(t, username)
        .await
        .log_server_error("unable to get templates owned by user", true)?; // should we handle the error case?
    tracing::debug!("user templates: {user_templates:?}");
    for temp in user_templates {
        templates.insert(temp.id(), temp);
    }

    let mut template_blobs = Vec::new();

    for pair in templates {
        let template = pair.1;
        let Template {
            id,
            name,
            deleted,
            description,
            owner,
            public,
            networks,
            hosts,
            lab,
        } = template;

        if !template.deleted {
            let mut host_blobs = Vec::new();
            let mut network_blobs = Vec::new();

            for hc in hosts {
                let ci = hc.clone().get_ci_file().await.unwrap();
                let HostConfig {
                    hostname,
                    flavor,
                    image,
                    cifile,
                    connections,
                } = hc;
                let port_profiles = flavor
                    .get(t)
                    .await
                    .log_db_client_error()?
                    .ports(t)
                    .await
                    .log_db_client_error()?;

                let mut bg_blobs = Vec::new();

                for BondGroupConfig {
                    connects_to,
                    member_interfaces,
                } in connections
                {
                    let mut networks = Vec::new();
                    let mut ifaces = Vec::new();

                    for VlanConnectionConfig { network, tagged } in connects_to {
                        let net = network.get(t).await.log_db_client_error()?.into_inner();

                        let cb = ConnectionBlob {
                            tagged,
                            connects_to: net.name,
                        };

                        networks.push(cb);
                    }

                    for iface_name in member_interfaces {
                        let ifp = port_profiles
                            .iter()
                            .find(|profile| profile.name == iface_name);
                        let ifb = InterfaceBlob {
                            name: iface_name,
                            speed: ifp.map(|p| p.speed).unwrap_or(DataValue {
                                value: 0,
                                unit: DataUnit::GigaBitsPerSecond,
                            }),
                            cardtype: ifp
                                .map(|p| p.cardtype)
                                .unwrap_or(models::inventory::CardType::Unknown),
                        };
                        ifaces.push(ifb);
                    }

                    let bgb = BondgroupBlob {
                        connections: networks,
                        ifaces,
                    };

                    bg_blobs.push(bgb);
                };


                let hcb = HostConfigBlob {
                    hostname,
                    flavor,
                    image,
                    cifile: ci,
                    bondgroups: bg_blobs,
                };
                host_blobs.push(hcb);
            }

            for maybe_net in networks.gotten(t).await {
                let Network { id, name, public } = maybe_net.log_db_client_error()?.into_inner();

                let nb = NetworkBlob { name, public };

                network_blobs.push(nb);
            }
            tracing::debug!("pushing template {id:?}");
            let lab = lab.get(t).await.expect("Expected to get origin lab");
            let tb = TemplateBlob {
                id: Some(id),
                owner: owner.unwrap_or("no owner".to_owned()),
                pod_name: name.clone(),
                pod_desc: description,
                public,
                host_list: host_blobs,
                networks: network_blobs,
                lab_name: lab.name.clone(),
            };

            tracing::debug!("Trying to add template: {name}");
            tracing::debug!(
                "template lab: {}, request lab: {}",
                lab.name,
                request_origin
            );
            if lab.name == request_origin {
                template_blobs.push(tb);
            }
        }
    }

    transaction.commit().await.log_db_client_error()?;
    Ok(Json(template_blobs))
}

#[axum::debug_handler]
pub async fn delete_template(Path(template_id): Path<ID>) -> Result<(), WebError> {
    tracing::info!("API call to delete_template()");
    let mut client = new_client().await.log_db_client_error()?;
    let mut transaction = client.easy_transaction().await.log_db_client_error()?;

    let mut existing_template = Template::get(&mut transaction, template_id)
        .await
        .log_server_error("unable to delete template", true)?;

    existing_template.deleted = true;
    existing_template
        .update(&mut transaction)
        .await
        .log_db_client_error()?;

    transaction.commit().await.log_db_client_error()?;

    Ok(())
}

pub async fn make_template(
    Path(lab_name): Path<String>,
    Json(blob): Json<TemplateBlob>,
) -> Result<Json<FKey<Template>>, WebError> {
    tracing::info!("API call to make_template()");
    let TemplateBlob {
        id,
        owner,
        pod_name,
        pod_desc,
        public,
        host_list,
        networks,
        lab_name,
    } = blob;

    // discard the id field, since it's meaningless in this context
    if let Some(v) = id {
        return Err("ID was provided for a templateblob, but this is meaningless since the template is still being created").anyway().log_error(StatusCode::BAD_REQUEST, "Unable to create a request with a requested id", false).expect("Expected to log error");
    }

    let mut client = new_client()
        .await
        .log_db_client_error()
        .expect("Expected to create a new client");

    let mut transaction = client
        .easy_transaction()
        .await
        .log_db_client_error()
        .expect("Expected to create a new transaction");

    let mut db_host_configs = Vec::new();

    let mut db_networks = Vec::new();

    let mut net_ids = HashMap::new();

    for NetworkBlob { name, public } in networks {
        let net_id: FKey<Network> = FKey::new_id_dangling();
        net_ids.insert(name.clone(), net_id);

        let network = NewRow::new(Network {
            id: net_id,
            name,
            public,
        });

        let id = network
            .insert(&mut transaction)
            .await
            .log_server_error("unable to insert network into db", true)
            .expect("Expected to log server error");

        db_networks.push(id);
    }

    for blob in host_list {
        let api::HostConfigBlob {
            hostname,
            flavor,
            image,
            cifile,
            bondgroups,
        } = blob;

        let mut bg_configs = Vec::new();

        for api::BondgroupBlob {
            connections: networks,
            ifaces,
        } in bondgroups.clone()
        {
            tracing::debug!("Adding a bondgroup with connections {networks:?} and ifaces {ifaces:?} for host {hostname} for a template that has been created");
            let mut bgc = dashboard::BondGroupConfig::default();

            for iface in ifaces.iter() {
                bgc.member_interfaces.insert(iface.name.clone());
            }

            for ConnectionBlob {
                tagged,
                connects_to,
            } in networks
            {
                let net_id = net_ids
                    .get(&connects_to)
                    .ok_or(format!(
                        "mismatched net IDs, couldn't find net by name {connects_to} in id map"
                    ))
                    .anyway()
                    .log_db_client_error()?;

                bgc.connects_to.insert(VlanConnectionConfig {
                    network: *net_id,
                    tagged,
                });
            }

            bg_configs.push(bgc);
        }

        info!("Making CI file with file: {:?}", cifile);

        let host = dashboard::HostConfig::new(hostname, flavor, image, cifile, bg_configs).await;

        db_host_configs.push(host);
    }

    let template = NewRow::new(Template {
        id: FKey::new_id_dangling(),
        name: pod_name,
        deleted: false,
        description: pod_desc,
        owner: Some(owner),
        public,
        networks: db_networks,
        hosts: db_host_configs,
        lab: Lab::get_by_name(&mut transaction, lab_name)
            .await
            .expect("Expected to find lab")
            .expect("Expected that lab exists")
            .id,
    });

    let template_fk = template
        .insert(&mut transaction)
        .await
        .log_db_client_error()?;

    transaction.commit().await.log_db_client_error()?;

    Ok(Json(template_fk))
}

pub fn routes(state: AppState) -> ApiRouter {
    ApiRouter::new()
        .route("/list/:lab_name/:user_id", get(list_templates))
        .route("/:template_id", delete(delete_template))
        .route("/:lab_name/create", post(make_template))
}
