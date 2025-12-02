use http::Uri;
use serde::{Deserializer, Serializer};

pub mod uri_vec_serde {
    use super::*;
    use serde::{Deserialize, Serialize};

    pub fn serialize<S>(uris: &[Uri], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let strings: Vec<String> = uris.iter().map(|uri| uri.to_string()).collect();
        strings.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<Uri>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let strings: Vec<String> = Vec::deserialize(deserializer)?;
        strings
            .into_iter()
            .map(|s| s.parse().map_err(serde::de::Error::custom))
            .collect()
    }
}

pub mod option_uri_serde {
    use super::*;
    use serde::Deserialize;

    pub fn serialize<S>(uri: &Option<Uri>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match uri {
            Some(u) => serializer.serialize_some(&u.to_string()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Uri>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<String> = Option::deserialize(deserializer)?;
        opt.map(|s| s.parse().map_err(serde::de::Error::custom))
            .transpose()
    }
}
