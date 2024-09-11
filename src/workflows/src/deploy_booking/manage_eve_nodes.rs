use common::prelude::{
    anyhow,
    hyper::StatusCode,
    reqwest::{self, Client},
};
use config::settings;
use models::dal::{AsEasyTransaction, EasyTransaction};
use models::{
    dal::{new_client, FKey},
    inventory::Host,
};
use serde_json::Value;
use tascii::prelude::*;

// tascii::mark_task!(OnboardEveNode);
// #[derive(Debug, Clone, Hash, Serialize, Deserialize)]
// pub struct OnboardEveNode {
//     user: String,
//     host_id: FKey<Host>,
// }

// impl AsyncRunnable for OnboardEveNode {
//     type Output = ();

//     async fn run(&mut self, context: &Context) -> Result<Self::Output, TaskError> {
//         let mut client = new_client().await?;
//         let mut transaction = client.easy_transaction().await?;

//         let mut client = connect().await;

//         let id = match get_node_id(&mut transaction, &mut client, self.host_id.clone()).await {
//             Ok(id) => id,
//             Err(e) => return Err(TaskError::Reason(e.to_string())),
//         };

//         // onnboarding certs
//         // /api/v1/devices/id/{id}/onboarding

//         // Create
//         // /api​/v1​/devices
//         let create_json: Value = serde_json::from_str("
//             {
//                 \"name\": {},
//                 \"title\": {},
//                 \"description\": {},
//                 \"projectId\": {},
//                 \"serialno\": {},
//                 \"location\": {},
//                 \"cpu\": 0,
//                 \"thread\": 0,
//                 \"memory\": 0,
//                 \"storage\": 0,
//                 \"onboarding\": {
//                     \"pemCert\": {},
//                     \"pemKey\": {}
//                 },
//                 \"identity\": {},

//                 \"clientIp\": {},
//                 \"modelId\": {},
//                 \"devLocation\": {
//                     \"underlayIP\": {},
//                     \"hostname\": {},
//                     \"city\": {},
//                     \"region\": {},
//                     \"country\": {},
//                     \"loc\": {},
//                     \"org\": {},
//                     \"postal\": {},
//                     \"latlong\": {},
//                     \"freeloc\": {}
//                 },
//                 \"generateSoftSerial\": false,
//         }
//         ").expect("Expected to serialize to json");

//         todo!()
//     }

//     fn identifier() -> TaskIdentifier {
//         todo!()
//     }

//     fn summarize(&self, id: models::dal::ID) -> String {
//         let task_ty_name = std::any::type_name::<Self>();
//         std::format!("Async Task {task_ty_name} with ID {id}")
//     }

//     fn variable_timeout(&self) -> std::time::Duration {
//         Self::timeout()
//     }

//     fn timeout() -> std::time::Duration {
//         std::time::Duration::from_secs_f64(120.0)
//     }

//     fn retry_count(&self) -> usize {
//         0
//     }
// }

tascii::mark_task!(DeleteEveNode);
#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct DeleteEveNode {
    pub host_id: FKey<Host>,
}

impl AsyncRunnable for DeleteEveNode {
    type Output = ();

    async fn run(&mut self, _context: &Context) -> Result<Self::Output, TaskError> {
        let mut client = new_client().await?;
        let mut transaction = client.easy_transaction().await?;

        let mut sandbox_client = connect().await;

        let id = match get_node_id(&mut transaction, &mut sandbox_client, self.host_id.clone()).await {
            Ok(id) => id,
            Err(e) => return Err(TaskError::Reason(e.to_string())),
        };

        let url = settings().eve.url.clone();

        let res = sandbox_client
            .post(format!("{url}/api/v1/devices/id/{id}/delete"))
            .header("Accept", "text/plain")
            .bearer_auth(settings().eve.api_key.clone())
            .send()
            .await;

        match res {
            Ok(r) => match r.status() {
                StatusCode::OK => Ok(()),
                StatusCode::UNAUTHORIZED => Err(TaskError::Reason("".to_owned())),
                StatusCode::FORBIDDEN => Err(TaskError::Reason(
                    "Unable to offboard node as we don't have access permissions to view it"
                        .to_owned(),
                )),
                StatusCode::NOT_FOUND => Err(TaskError::Reason(format!(
                    "Sandbox was unable to find the node with id {} to offboard it",
                    id.clone()
                ))),
                StatusCode::INTERNAL_SERVER_ERROR => Err(TaskError::Reason(
                    "Sandbox returned internal server error".to_owned(),
                )),
                StatusCode::GATEWAY_TIMEOUT => Err(TaskError::Reason(
                    "Sandbox returned gateway timeout".to_owned(),
                )),
                s => Err(TaskError::Reason(format!(
                    "unexpected status code {}",
                    s.as_str()
                ))),
            },
            Err(e) => Err(TaskError::Reason(format!(
                "Unable to reach sandbox instance {} due to error {}",
                url,
                e.to_string()
            ))),
        }
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("OffboardEveNode").versioned(1)
    }

    fn summarize(&self, id: models::dal::ID) -> String {
        let task_ty_name = std::any::type_name::<Self>();
        std::format!("Async Task {task_ty_name} with ID {id}")
    }

    fn variable_timeout(&self) -> std::time::Duration {
        Self::timeout()
    }

    fn timeout() -> std::time::Duration {
        std::time::Duration::from_secs_f64(120.0)
    }

    fn retry_count(&self) -> usize {
        0
    }
}

async fn connect() -> Client {
    reqwest::Client::builder()
        .cookie_store(true)
        .danger_accept_invalid_certs(true)
        .build()
        .expect("Expected to build client")
}

async fn get_node_id(
    transaction: &mut EasyTransaction<'_>,
    client: &mut Client,
    host_id: FKey<Host>,
) -> Result<String, anyhow::Error> {
    let host = host_id
        .get(transaction)
        .await
        .expect("Expected to find host");

    let url = settings().eve.url.clone();

    let res = client
        .post(format!("{}/api/v1/devices/serial/{}", url, host.serial.clone()))
        .header("Accept", "text/plain")
        .bearer_auth(settings().eve.api_key.clone())
        .send()
        .await;

    match res {
        Ok(r) => match r.status() {
            StatusCode::OK => match r.json::<Value>().await {
                Ok(v) => match v.as_object() {
                    Some(j) => match j.get("id") {
                        Some(strval) => match strval.as_str() {
                            Some(s) => Ok(s.to_owned()),
                            None => Err(anyhow::Error::msg(format!(
                                "Id field in response isn't a string: {:?}",
                                serde_json::to_string_pretty(strval)
                            ))),
                        },
                        None => Err(anyhow::Error::msg(format!(
                            "Unable to find id field in response: {:?}",
                            j
                        ))),
                    },
                    None => Err(anyhow::Error::msg(format!(
                        "Response is not json: {:?}",
                        serde_json::to_string_pretty(&v)
                    ))),
                },
                Err(_) => Err(anyhow::Error::msg("Unable to get response body")),
            },
            StatusCode::UNAUTHORIZED => Err(anyhow::Error::msg("")),
            StatusCode::FORBIDDEN => Err(anyhow::Error::msg(
                "Unable to get node id as we don't have access permissions to view it",
            )),
            StatusCode::NOT_FOUND => Err(anyhow::Error::msg(format!(
                "Sandbox was unable to find the node with serial number {}",
                host.serial.clone()
            ))),
            StatusCode::INTERNAL_SERVER_ERROR => {
                Err(anyhow::Error::msg("Sandbox returned internal server error"))
            }
            StatusCode::GATEWAY_TIMEOUT => {
                Err(anyhow::Error::msg("Sandbox returned gateway timeout"))
            }
            s => Err(anyhow::Error::msg(format!(
                "unexpected status code {}",
                s.as_str()
            ))),
        },
        Err(e) => Err(anyhow::Error::msg(format!(
            "Unable to reach sandbox instance {} due to error {}",
            url,
            e.to_string()
        ))),
    }
}
