use aide::{
    axum::{ApiRouter, IntoApiResponse},
    openapi::{Info, OpenApi, Tag},
    transform::TransformOpenApi,
};
use axum::{
    extract::{Json, Path},
    http::StatusCode,
    routing::{get, post},
    Extension,
};
use common::prelude::{
    aide, anyhow, axum, crossbeam_channel,
    itertools::Itertools,
    lazy_static,
    tracing::{self},
};
use crossbeam_channel::{Receiver, Sender};
use dal::{new_client, web::*, AsEasyTransaction, FKey, ID};
use models::dashboard::Instance;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tascii::prelude::{Runtime, Uuid};
use tracing::{error, info, warn};

use crate::{
    configure_networking::vlan_connection::create_network_manager_vlan_connections_from_bondgroups,
    deploy_booking::cloud_init::{render_meta_data, render_network_config, render_vendor_data},
    get_ipa_users,
};
// const MESSAGE_EXPIRY_TIME_MINUTES: f32 = 5.0;

#[derive(Serialize, Deserialize, Clone, Debug, Hash, JsonSchema, PartialEq, Eq)]
pub struct Message {
    pub id: ID,
    pub expired: bool,
    pub message: String,
}

#[derive(Clone, Default)]
pub struct AppState {
    _state: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Hash, PartialEq, Eq, JsonSchema)]
pub struct Endpoint {
    pub for_instance: FKey<Instance>,
    pub unique: ID, // unique for each constructed Endpoint, makes sure a host only hits the endpoint for the run that it's supposed to
}

impl Endpoint {
    pub fn to_url(&self) -> String {
        let path = config::settings().mailbox.external_url.clone();
        let for_host = self.for_instance.into_id().to_string();
        let unique = self.unique.to_string();

        format!("{path}/{for_host}/{unique}")
    }

    pub fn from_parts(for_instance: FKey<Instance>, id: ID) -> Self {
        Self {
            for_instance,
            unique: id,
        }
    }

    pub fn new(for_instance: FKey<Instance>) -> Self {
        Self::from_parts(for_instance, ID::new())
    }
}

pub type MailboxResult = Result<MailboxOk, MailboxErr>;

pub struct Mailbox {
    pub messages: HashMap<Endpoint, VecDeque<Message>>,

    pub un_acked: HashSet<Message>,

    pub notify_when: HashMap<Endpoint, (Sender<MailboxResult>, Receiver<MailboxResult>)>,
}

lazy_static::lazy_static! {
    static ref MAILBOX: Mutex<Mailbox> = Mutex::new(Mailbox::new());
}

#[derive(Clone, Debug)]
pub struct MailboxOk {
    pub msg: Message,
    pub endpoint: Endpoint,
}

#[derive(Clone, Debug)]
pub struct MailboxErr {
    pub failure_reason: String,
    pub endpoint: Endpoint,
}

pub struct MailboxMessageReceiver {
    recv: Receiver<MailboxResult>,
    received: Vec<MailboxResult>,

    for_endpoint: Endpoint,
}

impl MailboxMessageReceiver {
    pub fn wait_next(&mut self, timeout: Duration) -> MailboxResult {
        tracing::info!("Waiter is waiting on endpoint {:?}", self.for_endpoint);
        let v = self.recv.recv_timeout(timeout).map_err(|e| MailboxErr {
            failure_reason: format!("{e:?}"),
            endpoint: self.endpoint(),
        })?;

        tracing::warn!("Got a mailbox message: {v:?}");

        self.received.push(v.clone());

        tracing::info!("Added to received");

        if let Ok(m) = v.as_ref() {
            let mut mailbox = MAILBOX.lock().expect("couldn't lock mailbox");

            mailbox.un_acked.remove(&m.msg);
        }

        v
    }

    pub fn get_log(&self) -> &Vec<MailboxResult> {
        &self.received
    }

    pub fn endpoint(&self) -> Endpoint {
        self.for_endpoint
    }
}

impl std::ops::Drop for MailboxMessageReceiver {
    /// If a mailbox recver drops, then we
    /// assume we have gotten everything "from" it
    /// that we expect to, so we can have the mailbox
    /// close the corresponding endpoint/drop those messages
    fn drop(&mut self) {
        Mailbox::done_endpoint(self.endpoint());
    }
}

impl Default for Mailbox {
    fn default() -> Self {
        Self::new()
    }
}

impl Mailbox {
    pub fn new() -> Self {
        Self {
            messages: HashMap::new(),
            notify_when: HashMap::new(),

            un_acked: HashSet::new(),
        }
    }

    async fn clear(Path(endpoint): Path<Endpoint>) {
        let mut mailbox = MAILBOX.lock().expect("couldn't lock mailbox");
        mailbox.messages.remove(&endpoint);
    }

    pub async fn expired() {
        let _mailbox = MAILBOX.lock().expect("couldn't lock mailbox");
        // TODO: expired stuff
    }

    fn done_endpoint(endpoint: Endpoint) {
        let mut mailbox = MAILBOX.lock().expect("couldn't lock mailbox");

        mailbox.notify_when.remove(&endpoint);
    }

    /// push() is how we enter messages from a resource into the system
    /// use llid in path to signify our handle id (reconstruct endpoint using that)
    pub async fn push(Path((instance, llid)): Path<(FKey<Instance>, ID)>, msg: String) {
        warn!("Received message: {:#?}", msg);
        let mut mailbox = MAILBOX.lock().expect("couldn't lock mailbox");

        //this is not correct, endpoint needs to be reconstructed including the llid
        //so it can notify waits for the prior endpoint
        let endpoint = Endpoint::from_parts(instance, llid);
        info!("Created endpoint");
        mailbox.messages.insert(endpoint, VecDeque::new());
        let msg = Message {
            id: ID::new(),
            expired: false,
            message: msg,
        };
        info!("Created message");

        mailbox
            .messages
            .get_mut(&endpoint)
            .expect("Expected to find message queue.")
            .push_back(msg.clone());
        info!("Pushed message to queue");

        // need to iterate through entries in the vec for this endpoint's entry in notify_when,
        // push mbox::ok or mbox::err into each sender using send()
        info!("Informing listeners");
        let sender = mailbox.notify_when.entry(endpoint).or_insert_with(|| {
            info!("Had to create a new channel for it, no-one was waiting on the message yet? endpoint: {endpoint:?}");
            let (s, r) = crossbeam_channel::unbounded();

            (s, r)

        }).0.clone();

        info!("Sending Ok");
        let res = sender.send(Ok(MailboxOk { msg, endpoint }));
        match res {
            Err(_) => {
                error!("Error sending Ok")
            }
            Ok(_) => {
                info!("Ok sucessful")
            }
        }
        info!("Done with message!"); // made it to here

        /*let mut msg_queue = mailbox.messages.get_mut(&endpoint);
        let result = match msg_queue {
            None => {sender.send(Err(MailboxErr { failure_reason: "Failed to find message queue.".to_owned(), endpoint }))},
            Some(_) => {Ok(())}
        };

        if (msg_queue.is_some()) {
            let msg = msg_queue.unwrap().pop_front().expect("Expected to find message in queue.");
            sender.send(Ok(MailboxOk { msg, endpoint }));
        }*/
    }

    async fn peek(Path((instance, llid)): Path<(FKey<Instance>, ID)>) -> Json<Message> {
        let endpoint = Endpoint::from_parts(instance, llid);
        let mut mailbox = MAILBOX.lock().expect("couldn't lock mailbox");
        Json(
            mailbox
                .messages
                .get_mut(&endpoint)
                .expect("Expected to find message queue.")
                .front()
                .expect("Expected message at front")
                .clone(),
        )
    }

    // this isn't quite relevant anymore I don't think? since
    // we entirely interact with messages using the wait api
    //
    // I think pop and peek can both be removed
    async fn pop(Path((instance, llid)): Path<(FKey<Instance>, ID)>) -> Json<Message> {
        let endpoint = Endpoint::from_parts(instance, llid);
        let mut mailbox = MAILBOX.lock().expect("couldn't lock mailbox");
        Json(
            mailbox
                .messages
                .get_mut(&endpoint)
                .expect("Expected to find message queue.")
                .pop_front()
                .expect("Expected message at front"),
        )
    }

    pub fn waiter_for(endpoint: Endpoint) -> MailboxMessageReceiver {
        let mut mailbox = MAILBOX.lock().unwrap();

        let r = mailbox
            .notify_when
            .entry(endpoint)
            .or_insert_with(|| {
                let (s, r) = crossbeam_channel::unbounded();

                (s, r)
                /*let mut hmap = HashMap::new();
                hmap.insert(endpoint, s);
                hmap*/
            })
            .1
            .clone();

        MailboxMessageReceiver {
            recv: r,
            for_endpoint: endpoint,
            received: Vec::new(),
        }
    }

    pub fn endpoint_for(inst: FKey<Instance>) -> MailboxMessageReceiver {
        let endpoint = Endpoint::new(inst);

        Self::waiter_for(endpoint)
    }

    pub async fn set_endpoint_hook(
        instance: FKey<Instance>,
        usage: &'static str,
    ) -> Result<MailboxMessageReceiver, anyhow::Error> {
        let mut client = new_client().await?;
        let mut transaction = client.easy_transaction().await?;
        let waiter = Mailbox::endpoint_for(instance);
        let mut inst = instance.get(&mut transaction).await?;
        inst.metadata
            .insert(usage.to_owned(), serde_json::to_value(waiter.endpoint())?);
        inst.update(&mut transaction).await?;
        transaction.commit().await?;

        Ok(waiter)
    }

    pub async fn live_hooks(instance: FKey<Instance>) -> Result<Vec<String>, anyhow::Error> {
        let mut client = new_client().await?;
        let mut transaction = client.easy_transaction().await?;
        let inst = instance.get(&mut transaction).await?;

        let keys = inst.metadata.keys().cloned().collect_vec();

        transaction.commit().await?;

        Ok(keys)
    }

    pub async fn get_endpoint_hook(
        instance: FKey<Instance>,
        usage: &str,
    ) -> Result<Endpoint, anyhow::Error> {
        let mut client = new_client().await?;
        let mut transaction = client.easy_transaction().await?;
        let inst = instance.get(&mut transaction).await?;

        let hook = inst
            .metadata
            .get(usage)
            .cloned()
            .ok_or(anyhow::Error::msg("no matching hook found"))?;

        let hook: Endpoint = serde_json::from_value(hook)?;

        transaction.commit().await?;

        Ok(hook)
    }

    // this file should be complete and work properly
    // could at some point cache receivers and clone them instead of
    // doing multiple send, but that isn't important at the moment
    /*pub fn wait_next(endpoint: Endpoint) -> Result<MailboxOk, MailboxErr> {
        let mut mailbox = MAILBOX.lock().unwrap();

        //let (s, r) = crossbeam_channel::unbounded();

        let r =         r.recv().expect("the channel bronken")
    }*/
}

async fn get_user_data_file(
    Path((instance_id, _llid)): Path<(FKey<Instance>, ID)>,
) -> Result<impl IntoApiResponse, (StatusCode, String)> {
    let mut client = new_client().await.log_db_client_error()?;
    let mut transaction = client.easy_transaction().await.log_db_client_error()?;
    let instance = instance_id
        .get(&mut transaction)
        .await
        .expect("invalid hostconfig id provided");

    let host = instance
        .linked_host
        .unwrap_or_else(|| panic!("no host for requested ci file, instance_id = {instance_id:?}"));
    let host_name = host
        .get(&mut transaction)
        .await
        .expect("")
        .server_name
        .clone();
    info!("Host {host_name} requested user-cloud init file");

    match transaction.commit().await {
        Ok(_) => {}
        Err(e) => tracing::info!("Error in mailbox communication: {e:?}"),
    }

    match instance.config.clone().get_ci_file().await.unwrap() {
        Some(content) => Ok(content.to_owned()),
        None => {
            tracing::info!("Cloud init file for host {host_name} not found");
            Err((
                StatusCode::NOT_FOUND,
                "Cloud init user-data file not found".to_string()
            ))
        }
    }

}

async fn get_vendor_data_file(
    Path((instance_id, _llid)): Path<(FKey<Instance>, ID)>,
) -> Result<impl IntoApiResponse, (StatusCode, String)> {
    let mut client = new_client().await.log_db_client_error()?;
    let mut transaction = client.easy_transaction().await.log_db_client_error()?;

    let instance = instance_id
        .get(&mut transaction)
        .await
        .expect("invalid hostconfig id provided");

    let aggregate = instance
        .aggregate
        .get(&mut transaction)
        .await
        .expect("panic at getting instance aggregate");

    let ipa_users = get_ipa_users(aggregate.into_inner()).await;

    transaction.commit().await.unwrap();


    // Get post_provision mailbox so host can inform backend about state of cloud-init
    let mailbox_blob = instance.metadata.get("post_provision").unwrap();

    let mailbox_uuid: Uuid = serde_json::from_str(&mailbox_blob.get("unique").unwrap().to_string()).unwrap(); 

    let mailbox_unique = ID::from_str(&mailbox_uuid.to_string()).unwrap();

    let file_content = render_vendor_data(
        ipa_users,
        instance.config.hostname.clone(),
        Endpoint::from_parts(
            instance_id, // Mailbox instance should always be this instance
            mailbox_unique,
        ),
    );

    match file_content {
        Ok(content) => Ok(content.to_owned()),
        Err(_) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Error in retrieving cloud init file".to_string()
        )),
    }
}

async fn get_network_config_file(
    Path((instance_id, _llid)): Path<(FKey<Instance>, ID)>,
) -> Result<impl IntoApiResponse, (StatusCode, String)> {
    let mut client = new_client().await.log_db_client_error()?;
    let mut transaction = client.easy_transaction().await.log_db_client_error()?;

    let instance = instance_id
        .get(&mut transaction)
        .await
        .expect("Expected to get instance, most likely an invalid instance_id id was provided");

    let aggregate = instance
        .aggregate
        .get(&mut transaction)
        .await
        .expect("Expected to get instance aggregate");


    let host_ports = instance
        .linked_host
        .unwrap() // Kinda unsafe, prob good idea to redo
        .get(&mut transaction)
        .await
        .expect("Expected to get host information")
        .ports(&mut transaction)
        .await
        .expect("Expected to get host ports");


    let network_assignment_map = aggregate.vlans.get(&mut transaction).await.expect("Failed to Get Network Assignment Map");
    
    let file_content = render_network_config(
        host_ports,
        create_network_manager_vlan_connections_from_bondgroups(
            &network_assignment_map,
            &instance.config.connections,
            )
            .await
            .expect("Failed to generate vlan connections from bondgroups"),
    );

    match file_content {
        Ok(content) => Ok(content.to_owned()),
        Err(_) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Error in retrieving cloud init file".to_string()
        )),
    }
}

async fn get_meta_data_file(
    Path((instance_id, _llid)): Path<(FKey<Instance>, ID)>,
) -> Result<impl IntoApiResponse, (StatusCode, String)> {
    let file_content = render_meta_data(instance_id.into_id().into_uuid());

    match file_content {
        Ok(content) => Ok(content.to_owned()),
        Err(_) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Error in retrieving cloud init file".to_string()
        )),
    }
}


pub async fn entry(_rt: &'static Runtime) {
    let state = AppState::default();
    let mut api = OpenApi::default();

    let app = ApiRouter::new()
        .route("/", get(test))
        .route("/clear/:msg_id", post(Mailbox::clear))
        .route("/expired", get(Mailbox::expired))
        .route("/:instance/:unique/push", post(Mailbox::push))
        .route("/:instance/:unique/peek", post(Mailbox::peek))
        .route("/:instance/:unique/pop", post(Mailbox::pop))
        .route("/:instance/:unique/cloud-init/user-data", get(get_user_data_file))
        .route("/:instance/:unique/cloud-init/vendor-data", get(get_vendor_data_file))
        .route("/:instance/:unique/cloud-init/network-config", get(get_network_config_file))
        .route("/:instance/:unique/cloud-init/meta-data", get(get_meta_data_file))
        .finish_api_with(&mut api, api_docs)
        .layer(Extension(Arc::new(api)))
        .with_state(state);

    let _api = OpenApi {
        info: Info {
            description: Some("Booking API".to_string()),
            ..Info::default()
        },
        ..OpenApi::default()
    };

    fn api_docs(api: TransformOpenApi) -> TransformOpenApi {
        api.title("LibLaaS-Mailbox API")
            .summary("Provides mailbox for host provisioning.")
            .description("")
            .tag(Tag {
                name: "LibLaaS-Mailbox".into(),
                description: Some("LibLaaS management".into()),
                ..Default::default()
            })
            .security_scheme(
                "Apikey",
                aide::openapi::SecurityScheme::ApiKey {
                    location: aide::openapi::ApiKeyLocation::Header,
                    name: "X-Auth-Key".into(),
                    description: Some("Key from dashboard".to_string()),
                    extensions: Default::default(),
                },
            )
            .default_response_with::<Json<ApiError>, _>(|res| {
                res.example(ApiError::trivial(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Default error, something has gone wrong".to_string(),
                ))
            })
    }

    let mailbox_addr = config::settings().mailbox.bind_addr.clone();
    tracing::info!("Binding to {}", mailbox_addr.to_string());
    let _res = axum::Server::bind(
        &std::net::SocketAddr::from_str(&mailbox_addr.to_string())
            .expect("Expected api address as a string."),
    )
    .serve(app.into_make_service())
    .await;
}

// async fn serve_api(Extension(api): Extension<OpenApi>) -> impl IntoApiResponse {
//      Json(api)
// }

async fn test() -> String {
    "Test :".to_owned()
}
