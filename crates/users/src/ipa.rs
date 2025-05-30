use ::serde::{Deserialize, Serialize};
use axum::http::HeaderValue;
use common::prelude::{
    anyhow::{self},
    async_recursion::async_recursion,
    config::*,
    reqwest,
    reqwest::tls::Certificate,
    serde_json,
    serde_with::skip_serializing_none,
    strum_macros::Display,
    tracing,
};
use schemars::{
    JsonSchema,
    _serde_json::{json, Value},
};
use strum_macros::EnumIter;

use crate::anyhow::anyhow;

use std::{collections::HashMap, fs::read, path::PathBuf};
pub struct IPA {
    client: reqwest::Client,
    id: u32,
    version: String,
    ipa: IPAConfig,
}

pub enum UserCreationFailure {
    AlreadyExists(),
    ConnectionFailure(),
    AuthenticationFailure(),
}

#[allow(non_camel_case_types)] // To allow names to match the expected IPA names
#[derive(Serialize, Deserialize, Display, Debug, Clone, Hash, EnumIter)]
pub enum UserData {
    uid(Option<String>),
    /// Username
    givenname(Option<String>), //First name
    sn(Option<String>),
    /// Last name
    cn(Option<String>),
    /// Full name
    displayname(Option<String>),
    homedirectory(Option<PathBuf>),
    loginshell(Option<PathBuf>),
    mail(Option<String>),
    /// user email
    userpassword(Option<String>),
    uidnumber(Option<i32>),
    gidnumber(Option<i32>),
    ou(Option<String>),
    ipasshpubkey(Option<String>),
    ipauserauthtype(Option<String>),
    userclass(Option<String>),
    usercertificate(Option<String>),
    /// cert
    rename(Option<String>),
}

/// Allows for the use of enums while interacting with user data and their strange naming to prevent errors
/// This is nasty (for now) as there's not an easy way to remove generic data from enum variants
/// and would be better as a macro, will discuss later
impl UserData {
    pub fn get_data_string(var: UserData) -> String {
        match var.clone() {
            // TODO: These should just have different impls for these types, the approach really
            // REALLY needs reconsideration
            // String, i32, PathBuf
            UserData::uid(d)
            | UserData::givenname(d)
            | UserData::sn(d)
            | UserData::cn(d)
            | UserData::displayname(d)
            | UserData::ou(d)
            | UserData::ipasshpubkey(d)
            | UserData::ipauserauthtype(d)
            | UserData::userclass(d)
            | UserData::usercertificate(d)
            | UserData::rename(d)
            | UserData::mail(d)
            | UserData::userpassword(d) => {
                if let Some(value) = d {
                    value
                } else {
                    "".to_owned()
                }
            }
            UserData::homedirectory(d) | UserData::loginshell(d) => {
                if let Some(path_buf) = d {
                    path_buf
                        .clone()
                        .to_str()
                        .expect("Expected pathbuf to be valid")
                        .to_owned()
                } else {
                    "".to_owned()
                }
            }

            UserData::uidnumber(d) | UserData::gidnumber(d) => {
                if let Some(num) = d {
                    num.to_string()
                } else {
                    "".to_owned()
                }
            }
        }
    }
}

#[skip_serializing_none]
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Hash)]
pub struct User {
    pub uid: String,        //Username
    pub givenname: String,  //First name
    pub sn: String,         //Last name
    pub cn: Option<String>, //Full name
    pub homedirectory: Option<PathBuf>,
    pub gidnumber: Option<String>,
    pub displayname: Option<String>,
    pub loginshell: Option<PathBuf>,
    pub mail: String, //user email
    pub userpassword: Option<String>,
    pub random: Option<bool>, //Random user pass
    pub uidnumber: Option<String>,
    pub ou: String,
    pub title: Option<String>,
    pub ipasshpubkey: Option<Vec<String>>,
    pub ipauserauthtype: Option<String>,
    pub userclass: Option<String>,
    pub usercertificate: Option<String>, //cert
}

impl IPA {
    pub async fn init() -> Result<IPA, anyhow::Error> {
        //Vec of all servers provided
        let ipa_vec = &settings().ipa;
        //connection_error: error that will be returned by the fucntion if connection is unsuccessful
        let mut connection_error: anyhow::Error =
            anyhow::Error::msg("No server URLs in config, unable to connect");
        for current_ipa in ipa_vec {
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                "Referer",
                HeaderValue::from_str(format!("{}/ipa", current_ipa.url).as_str()).unwrap(),
            );

            let client = reqwest::Client::builder()
                .add_root_certificate(
                    Certificate::from_pem(
                        &read(current_ipa.certificate_path.clone()).expect("Expected to find file"),
                    )
                    .expect("Expected to get cert"),
                )
                .default_headers(headers)
                .cookie_store(true)
                .danger_accept_invalid_certs(true)
                .build()
                .expect("Expected to build client");
            // new_ipa: This will be created and saved as an IPA if the connection is succssful

            let mut new_ipa = IPA {
                client,
                id: 0,
                version: "2.245".to_owned(),
                ipa: current_ipa.clone(),
            };
            let res_ipa = new_ipa.get_auth().await;
            match res_ipa {
                Ok(i) => return Ok(new_ipa),
                Err(e) => connection_error = e,
            }
        }
        Err(anyhow::Error::msg(format!(
            "Failed to connect to any ipa server due to {}",
            connection_error
        )))
    }

    pub async fn get_auth(&mut self) -> Result<bool, anyhow::Error> {
        let user = self.ipa.username.as_str();
        let password = self.ipa.password.as_str();

        let form: HashMap<&str, &str> = [("user", user), ("password", password)]
            .into_iter()
            .collect();

        let res = self
            .client
            .post(format!("{}/ipa/session/login_password", self.ipa.url))
            .header("Accept", "text/plain")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&form)
            .send()
            .await;

        match res {
            Ok(r) => match r.text().await {
                Ok(text) => {
                    if text.is_empty() {
                        Ok(true)
                    } else {
                        match json!(text).as_object() {
                            Some(j) => {
                                match j
                                    .get("response")
                                    .unwrap()
                                    .as_object()
                                    .unwrap()
                                    .get("status")
                                    .unwrap()
                                    .as_i64()
                                    .unwrap()
                                {
                                    i if (200..=299).contains(&i) => Ok(true),
                                    _ => Err(anyhow::Error::msg(format!(
                                        "Failed to authenticate, got: {text:#?}"
                                    ))),
                                }
                            }
                            None => Err(anyhow::Error::msg(format!(
                                "Received a non-json value: {text:#?}"
                            ))),
                        }
                    }
                }
                Err(e) => Err(anyhow::Error::msg(e.to_string())),
            },
            Err(e) => Err(anyhow::Error::msg(e.to_string())),
        }
    }

    /// Returns the created user
    #[async_recursion]
    pub async fn create_user(&mut self, user: User, run_once: bool) -> Result<User, anyhow::Error> {
        let id = self.id;
        let mut g = json!(user).as_object_mut().unwrap().clone();
        g.insert("all".to_owned(), serde_json::Value::Bool(true));
        g.insert(
            "version".to_owned(),
            serde_json::Value::String(self.version.clone()),
        );

        let json = json!({
            "method": "user_add",
            "params": [
                [g.remove("uid").expect("Expected uid to exist")],
                g
            ],
            "id": id,
        });

        self.id += 1;

        let res = self
            .client
            .post(format!("{}/ipa/session/json", self.ipa.url))
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&json)
            .send()
            .await;
        match res {
            Ok(r) => match r.status() {
                axum::http::StatusCode::UNAUTHORIZED => {
                    let res = self.get_auth().await;
                    if let Err(e) = res {
                        return Err(anyhow::Error::msg(e));
                    }
                    if !run_once {
                        return self.create_user(user, true).await;
                    } else {
                        return Err(anyhow::Error::msg("Failed to authenticate"));
                    }
                }
                _ => {
                    let value: Value = r.json().await.unwrap();
                    let object = value
                        .as_object()
                        .unwrap()
                        .get("result")
                        .unwrap()
                        .as_object()
                        .unwrap()
                        .get("result")
                        .unwrap();
                    return Ok(User {
                        uid: object
                            .get("uid")
                            .unwrap()
                            .as_array()
                            .unwrap()
                            .first()
                            .unwrap()
                            .as_str()
                            .unwrap()
                            .to_owned(),
                        givenname: object
                            .get("givenname")
                            .unwrap()
                            .as_array()
                            .unwrap()
                            .first()
                            .unwrap()
                            .as_str()
                            .unwrap()
                            .to_owned(),
                        sn: object
                            .get("sn")
                            .unwrap()
                            .as_array()
                            .unwrap()
                            .first()
                            .unwrap()
                            .as_str()
                            .unwrap()
                            .to_owned(),
                        cn: object.get("cn").map(|value| {
                            value
                                .as_array()
                                .unwrap()
                                .first()
                                .unwrap()
                                .as_str()
                                .unwrap()
                                .to_owned()
                        }),
                        homedirectory: object.get("homedirectory").map(|value| {
                            value
                                .as_array()
                                .unwrap()
                                .first()
                                .unwrap()
                                .as_str()
                                .unwrap()
                                .into()
                        }),
                        gidnumber: object.get("gidnumber").map(|value| {
                            value
                                .as_array()
                                .unwrap()
                                .first()
                                .unwrap()
                                .as_str()
                                .unwrap()
                                .to_owned()
                        }),
                        displayname: object.get("displayname").map(|value| {
                            value
                                .as_array()
                                .unwrap()
                                .first()
                                .unwrap()
                                .as_str()
                                .unwrap()
                                .to_owned()
                        }),
                        loginshell: object.get("loginshell").map(|value| {
                            value
                                .as_array()
                                .unwrap()
                                .first()
                                .unwrap()
                                .as_str()
                                .unwrap()
                                .into()
                        }),
                        mail: object
                            .get("mail")
                            .unwrap()
                            .as_array()
                            .unwrap()
                            .first()
                            .unwrap()
                            .as_str()
                            .unwrap()
                            .to_owned(),
                        userpassword: object
                            .get("randompassword")
                            .map(|value| value.as_str().unwrap().to_owned()),
                        random: None,
                        uidnumber: object.get("uidnumber").map(|value| {
                            value
                                .as_array()
                                .unwrap()
                                .first()
                                .unwrap()
                                .as_str()
                                .unwrap()
                                .to_owned()
                        }),
                        ou: object
                            .get("ou")
                            .unwrap()
                            .as_array()
                            .unwrap()
                            .first()
                            .unwrap()
                            .as_str()
                            .unwrap()
                            .to_owned(),
                        title: object.get("title").map(|value| {
                            value
                                .as_array()
                                .unwrap()
                                .first()
                                .unwrap()
                                .as_str()
                                .unwrap()
                                .to_owned()
                        }),
                        ipasshpubkey: None, // Too complicated to parse out and not useful
                        ipauserauthtype: object.get("ipauserauthtype").map(|value| {
                            value
                                .as_array()
                                .unwrap()
                                .first()
                                .unwrap()
                                .as_str()
                                .unwrap()
                                .to_owned()
                        }),
                        userclass: object.get("userclass").map(|value| {
                            value
                                .as_array()
                                .unwrap()
                                .first()
                                .unwrap()
                                .as_str()
                                .unwrap()
                                .to_owned()
                        }),
                        usercertificate: object.get("usercertificate").map(|value| {
                            value
                                .as_array()
                                .unwrap()
                                .first()
                                .unwrap()
                                .as_str()
                                .unwrap()
                                .to_owned()
                        }),
                    });
                }
            },
            Err(e) => Err(anyhow::Error::msg(e)),
        }
    }

    #[async_recursion]
    pub async fn find_matching_user(
        &mut self,
        username: String,
        all: bool,
        run_once: bool,
    ) -> Result<User, anyhow::Error> {
        let json = json!({
            "method": "user_show",
            "params": [
                [username],
                {
                    "all": all,
                    "version": self.version
                }
            ],
            "id": self.id,
        });
        self.id += 1;
        tracing::debug!("req\n{}", json);

        let res = self
            .client
            .post(format!("{}/ipa/session/json", self.ipa.url))
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&json)
            .send()
            .await;
        tracing::debug!("Res: {res:#?}");
        let text = match res {
            Ok(r) => match r.status() {
                axum::http::StatusCode::UNAUTHORIZED => {
                    let res = self.get_auth().await;
                    tracing::debug!("Reauth res: {res:#?}");
                    if let Err(e) = res {
                        return Err(anyhow::Error::msg(e.to_string()));
                    }
                    if !run_once {
                        return self.find_matching_user(username, all, true).await;
                    } else {
                        return Err(anyhow::Error::msg("Failed to authenticate"));
                    }
                }
                _ => match r.text().await {
                    Ok(s) => {
                        tracing::debug!("resp\n{}", s);
                        s
                    }
                    Err(e) => return Err(anyhow::Error::msg(e.to_string())),
                },
            },
            Err(e) => {
                return Err(anyhow::Error::msg(e.to_string()));
            }
        };
        let v: Result<serde_json::Value, _> = serde_json::from_str(text.as_str());
        match v.unwrap().as_object_mut() {
            Some(u) => {
                if u.get_key_value("error").is_none() {
                    return Err(anyhow::Error::msg(u.get("error").unwrap().to_string()));
                }

                let err = format!("Bad structure from ipa response, got {u:?}");

                let ud = u
                    .get_mut("result")
                    .ok_or(anyhow::Error::msg(err.clone()))?
                    .as_object_mut()
                    .ok_or(anyhow::Error::msg(err.clone()))?
                    .get_mut("result")
                    .ok_or(anyhow::Error::msg(err.clone()))?
                    .as_object()
                    .ok_or(anyhow::Error::msg(err.clone()))?;

                let ud: HashMap<_, Value> = ud
                    .into_iter()
                    .map(|(key, value)| {
                        if value.is_array() && key.ne("ipasshpubkey") {
                            (
                                key.clone(),
                                value.as_array().unwrap().first().unwrap().clone(),
                            )
                        } else {
                            (key.clone(), value.clone())
                        }
                    })
                    .collect();

                let user = User {
                    uid: username,
                    givenname: ud.get("givenname").unwrap().as_str().unwrap().to_owned(),
                    sn: ud.get("sn").unwrap().as_str().unwrap().to_owned(),
                    cn: if ud.contains_key("cn") {
                        Some(ud.get("cn").unwrap().as_str().unwrap().to_owned())
                    } else {
                        None
                    },
                    homedirectory: if ud.contains_key("homedirectory") {
                        Some(PathBuf::from(
                            ud.get("homedirectory")
                                .unwrap()
                                .as_str()
                                .to_owned()
                                .unwrap(),
                        ))
                    } else {
                        None
                    },
                    gidnumber: match ud.contains_key("gidnumber") {
                        true => Some(ud.get("gidnumber").unwrap().as_str().unwrap().to_owned()),
                        false => None,
                    },
                    displayname: if ud.contains_key("displayname") {
                        Some(ud.get("displayname").unwrap().as_str().unwrap().to_owned())
                    } else {
                        None
                    },
                    loginshell: if ud.contains_key("loginshell") {
                        Some(PathBuf::from(
                            ud.get("loginshell").unwrap().as_str().to_owned().unwrap(),
                        ))
                    } else {
                        None
                    },
                    mail: ud.get("mail").unwrap().as_str().unwrap().to_owned(),
                    userpassword: if ud.contains_key("userpassword") {
                        Some(ud.get("userpassword").unwrap().as_str().unwrap().to_owned())
                    } else {
                        None
                    },
                    random: if ud.contains_key("random") {
                        Some(ud.get("random").unwrap().as_bool().unwrap())
                    } else {
                        None
                    },
                    uidnumber: if ud.contains_key("uidnumber") {
                        Some(ud.get("uidnumber").unwrap().as_str().unwrap().to_owned())
                    } else {
                        None
                    },
                    ou: if ud.contains_key("ou") {
                        ud.get("ou").unwrap().as_str().unwrap().to_owned()
                    } else {
                        "".to_string()
                    },
                    title: if ud.contains_key("title") {
                        Some(ud.get("title").unwrap().as_str().unwrap().to_owned())
                    } else {
                        None
                    },
                    ipasshpubkey: if ud.contains_key("ipasshpubkey") {
                        Some(
                            ud.get("ipasshpubkey")
                                .unwrap()
                                .as_array()
                                .unwrap()
                                .clone()
                                .into_iter()
                                .map(|s| s.as_str().unwrap().to_owned())
                                .collect(),
                        )
                    } else {
                        None
                    },
                    ipauserauthtype: if ud.contains_key("ipauserauthtype") {
                        Some(
                            ud.get("ipauserauthtype")
                                .unwrap()
                                .as_str()
                                .unwrap()
                                .to_owned(),
                        )
                    } else {
                        None
                    },
                    userclass: if ud.contains_key("userclass") {
                        Some(ud.get("userclass").unwrap().as_str().unwrap().to_owned())
                    } else {
                        None
                    },
                    usercertificate: if ud.contains_key("usercertificate") {
                        Some(
                            ud.get("usercertificate")
                                .unwrap()
                                .as_str()
                                .unwrap_or_default()
                                .to_owned(),
                        )
                    } else {
                        None
                    },
                };
                //proc_ud.insert("uid".to_owned(), serde_json::Value::String(username));
                tracing::info!("user: {:#?}", user);
                // let user: User = match serde_json::from_value(serde_json::Value::from(proc_ud)) {
                //     Ok(u) => u,
                //     Err(e) => return Err(anyhow::Error::msg(format!("User doesn't exist, err: {e:#?}"))),
                // };
                Ok(user)
            }
            None => Err(anyhow::Error::msg("Failed to serialize response to user")),
        }
    }

    #[async_recursion]
    pub async fn update_user(
        &mut self,
        username: String,
        add_data: Vec<UserData>,
        new_data: Vec<UserData>,
        run_once: bool,
    ) -> Result<bool, anyhow::Error> {
        let mut add: Vec<String> = Vec::new();
        let mut set: Vec<String> = Vec::new();
        let mut del: Vec<String> = Vec::new();

        for data in new_data.clone() {
            if UserData::get_data_string(data.clone()).is_empty()
                && data.clone().to_string() != "ipasshpubkey"
            {
                del.push(format!("{}=\"\"", data.clone()))
            } else {
                set.push(format!(
                    "{}={}",
                    data.clone(),
                    UserData::get_data_string(data)
                ))
            }
        }

        for data in add_data.clone() {
            add.push(format!(
                "{}={}",
                data.clone(),
                UserData::get_data_string(data)
            ))
        }
        // tracing::info!("set: {:?}", set);
        // tracing::info!("del: {:?}", del);

        let mut params = json!({}).as_object_mut().unwrap().clone();

        if !set.is_empty() {
            let set_val: Vec<Value> = set.into_iter().map(Value::String).collect();
            params.insert("setattr".to_owned(), Value::Array(set_val));
        }

        if !del.is_empty() {
            let del_val: Vec<Value> = del.into_iter().map(Value::String).collect();
            params.insert("delattr".to_owned(), Value::Array(del_val));
        }

        if !add.is_empty() {
            let add_val: Vec<Value> = add.into_iter().map(Value::String).collect();
            params.insert("addattr".to_owned(), Value::Array(add_val));
        }

        params.insert("all".to_string(), serde_json::Value::Bool(true));
        params.insert(
            "version".to_string(),
            serde_json::Value::String(self.version.to_owned()),
        );

        // tracing::info!("params: {:?}", params);

        let json = json!({
            "method": "user_mod",
            "params": [
                [username],
                params
            ],
            "id": self.id,
        });

        // tracing::info!(
        //     "request\n{}",
        //     serde_json::to_string_pretty(&json).expect("Expected to get a pretty string")
        // );
        self.id += 1;

        let res = self
            .client
            .post(format!("{}/ipa/session/json", self.ipa.url))
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&json)
            .send()
            .await;

        match res {
            Ok(r) => match r.status() {
                axum::http::StatusCode::UNAUTHORIZED => {
                    let res = self.get_auth().await;
                    if let Err(e) = res {
                        return Err(anyhow::Error::msg(e.to_string()));
                    }
                    if !run_once {
                        return self.update_user(username, add_data, new_data, true).await;
                    } else {
                        return Err(anyhow::Error::msg("Failed to authenticate"));
                    }
                }
                _ => {
                    // tracing::info!("status\n{}", r.status());
                    let j: Result<Value, _> = r.json().await;
                    match j {
                        Ok(val) => {
                            if val
                                .as_object()
                                .expect("expected json to be json")
                                .get_key_value("error")
                                .is_none()
                            {
                                return Err(anyhow::Error::msg(
                                    val.as_object().unwrap().get("error").unwrap().to_string(),
                                ));
                            } else {
                                return Ok(true);
                            }
                        }
                        Err(e) => Err(anyhow::Error::msg(e.to_string())),
                    }
                }
            },
            Err(e) => Err(anyhow::Error::msg(e.to_string())),
        }
    }

    pub async fn group_add_user(
        &mut self,
        group_name: &String,
        user: &String,
    ) -> Result<bool, anyhow::Error> {
        self.group_mod_user("group_add_member", group_name, user, true)
            .await
    }

    pub async fn group_remove_user(
        &mut self,
        group_name: &String,
        user: &String,
    ) -> Result<bool, anyhow::Error> {
        self.group_mod_user("group_remove_member", group_name, user, true)
            .await
    }

    /**
     * Finds groups for a given user
     * Returns ALL groups, not just managed ones
     * Make sure to only add / remove groups that are managed by liblaas
     */
    pub async fn group_find_user(
        &mut self,
        username: &String,
    ) -> Result<Vec<String>, anyhow::Error> {
        let mut user_groups: Vec<String> = vec![];

        // ipa group-find --user=username
        let json = json!({
            "method": "group_find",
            "params": [
                [],
                {
                    "user": username
                }
            ],
            "id": self.id,
        });

        self.id += 1;

        let res = self
            .client
            .post(format!("{}/ipa/session/json", self.ipa.url))
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&json)
            .send()
            .await;

        let text = match res {
            Ok(r) => r.text().await?,
            Err(e) => return Err(anyhow::Error::msg(e.to_string())),
        };

        let text_json: serde_json::Value = serde_json::from_str(text.as_str())?;
        let error = text_json.get("error").unwrap();

        if !error.is_null() {
            return Err(anyhow!("IPA returned an error! {error:?}"));
        }

        let result = text_json.get("result").unwrap().get("result").unwrap();

        if !result.is_array() {
            return Err(anyhow!("groups_find_user did not return an array!"));
        }

        for r in result.as_array().unwrap() {
            // "cn" is the field that contains the name of the group. It is represented as an array of strings.
            let group_name = r
                .get("cn")
                .unwrap()
                .as_array()
                .unwrap()
                .first()
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned();
            user_groups.push(group_name);
        }

        Ok(user_groups)
    }

    #[async_recursion]
    pub async fn group_mod_user(
        &mut self,
        action: &str,
        group_name: &String,
        user: &String,
        run_once: bool,
    ) -> Result<bool, anyhow::Error> {
        let json = json!({
            "method": action,
            "params": [
                [group_name],
                {
                    "user": user,
                    "version": self.version
                }
            ],
            "id": self.id,
        });
        self.id += 1;

        let res = self
            .client
            .post(format!("{}/ipa/session/json", self.ipa.url))
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&json)
            .send()
            .await;

        let text = match res {
            Ok(r) => match r.status() {
                axum::http::StatusCode::UNAUTHORIZED => {
                    let res = self.get_auth().await;
                    if let Err(e) = res {
                        return Err(anyhow::Error::msg(e.to_string()));
                    }
                    if !run_once {
                        return self.group_mod_user(action, group_name, user, true).await;
                    } else {
                        return Err(anyhow::Error::msg("Failed to authenticate"));
                    }
                }
                _ => match r.text().await {
                    Ok(s) => s,
                    Err(e) => return Err(anyhow::Error::msg(e.to_string())),
                },
            },
            Err(e) => {
                return Err(anyhow::Error::msg(e.to_string()));
            }
        };
        let v: Result<serde_json::Value, _> = serde_json::from_str(text.as_str());
        match v.unwrap().as_object_mut() {
            Some(u) => {
                if u.get_key_value("error").is_none() {
                    Err(anyhow::Error::msg(u.get("error").unwrap().to_string()))
                } else {
                    Ok(true)
                }
            }
            None => Err(anyhow::Error::msg("Failed to serialize response to user")),
        }
    }
}

// [-] creation
// [-] deletion?
// [-] detail management
// [-] group management
// ssh
// [ ] upload
// [ ] download
// [ ] query
