//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT
use common::prelude::chrono::{DateTime, Utc};
use common::prelude::tokio_postgres;
use models::dashboard::NetworkBlob;
use std::borrow::{Borrow, BorrowMut};
use std::marker::PhantomData;
use std::net::{Ipv4Addr, Ipv6Addr};
use strum_macros::Display;
use tokio_postgres::types::{BorrowToSql, ToSql};

use models::{
    dal::FKey,
    dashboard::{Aggregate, Image, Template},
    inventory::{self, CardType, DataValue, Flavor},
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use common::prelude::*;

pub struct AggregateBlob {
    with_template: FKey<Template>,
}

///////////////////////////////////////////////////////////////////////////////////////////////////
/// Any struct declared as a 'blob' is a dashboard friendly struct
/// to be sent over the API, but should never be directly stored into any database
/// They are fundamentally ephemeral, and describe the shape of the API

/// The highest level blob containing all the neccessary information to create a template
/// Dashboard sends TemplateBlob
#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
pub struct TemplateBlob {
    pub id: Option<FKey<Template>>,
    ///
    pub owner: String,

    pub lab_name: String,
    ///
    pub pod_name: String,
    ///
    pub pod_desc: String,
    ///
    pub public: bool,
    ///
    pub host_list: Vec<HostConfigBlob>,
    ///
    pub networks: Vec<NetworkBlob>,
}

/// Lower level blob containing the configuration for a single host in a template
#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
pub struct HostConfigBlob {
    /// Hostname entered by the user
    pub hostname: String,
    /// UUID of the selected flavor
    pub flavor: FKey<Flavor>,
    /// UUID of the selected image
    pub image: FKey<Image>,
    /// A vector of C-I Files. order is determined by order of the Vec
    pub cifile: Vec<String>,
    ///
    pub bondgroups: Vec<BondgroupBlob>,
}

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
pub struct HostBlob {
    pub id: Option<FKey<inventory::Host>>,
    pub name: String,
    pub arch: String,
    pub flavor: FKey<inventory::Flavor>,
    pub ipmi_fqdn: String,
    pub allocation: Option<AllocationBlob>,
}

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
pub struct AggregateDescription {
    pub id: FKey<Aggregate>,
    pub purpose: Option<String>,
    pub project: Option<String>,
    pub origin: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
pub struct AllocationBlob {
    pub for_aggregate: Option<AggregateDescription>,
    pub reason: String,
}

/// corresponds to aggregation groups within the backplane,
/// allows grouping several host ports into a single virtual interface
#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
pub struct BondgroupBlob {
    pub connections: Vec<ConnectionBlob>,
    pub ifaces: Vec<InterfaceBlob>,
}

/// Denotes the connection of an aggregated interface
/// to a network, and whether it should be talking
/// tagged frames or untagged frames
#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
pub struct ConnectionBlob {
    /// Whether the connection is using tagged frames
    pub tagged: bool,

    /// the name of the network this connects to
    pub connects_to: String,
}

/// Dashboard friendly information about an Image
#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
pub struct ImageBlob {
    /// UUID of associated image
    pub image_id: FKey<Image>,
    ///
    pub name: String,
}

/// Workflow friendly representation of a Flavor
#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
pub struct FlavorBlob {
    /// UUID of associated Flavor
    pub flavor_id: FKey<Flavor>,
    ///
    pub name: String,
    ///
    pub interfaces: Vec<InterfaceBlob>,

    pub images: Vec<ImageBlob>,

    pub available_count: usize,
    pub cpu_count: usize,     // Max 4.294967295 Billion
    pub ram: DataValue,       // Max 4.294 Petabytes in gig
    pub root_size: DataValue, // Max 4.294 Exabytes in gig
    pub disk_size: DataValue, // Max 4.294 Exabytes in gig
    pub swap_size: DataValue, // Max 9.223372036854775807 Exabytes in gig
}

/// Full details of an interface on a given Flavor
#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
pub struct InterfaceBlob {
    pub name: String,

    pub speed: DataValue,

    pub cardtype: CardType,
}

#[derive(Serialize, Deserialize, Debug, JsonSchema, Clone)]
pub struct BookingBlob {
    /// The originating project for this booking
    pub origin: String,
    /// UUID of selected Template
    pub template_id: FKey<Template>,
    /// The set of additional people (IPA usernames) who should have access (VPN, SSH) access in this booking
    pub allowed_users: Vec<String>,
    /// Global CI file override
    pub global_cifile: String,
    /// Metadata for a booking blob, differing from the ideal values will cause gaps in notification data sent to users
    pub metadata: BookingMetadataBlob,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct BookingMetadataBlob {
    /// The dashboard booking id
    pub booking_id: Option<String>,
    /// The ipa username of the owner of the booking
    pub owner: Option<String>,
    /// The lab a booking is for
    pub lab: Option<String>,
    /// The purpose of a booking
    pub purpose: Option<String>,
    /// Project a booking belongs to
    pub project: Option<String>,
    /// The length in days of a booking
    pub length: Option<u64>,
}

pub mod user_management {

    pub struct LFUser {
        lf_username: LFUserName,
        lf_email_address: String,
    }

    pub struct SSHKey {
        raw: String,
        title: String,
    }

    pub struct IPAUser {
        ipa_username: IPAUserName,
        email_addresses: Vec<String>,
        first_name: Option<String>,
        last_name: Option<String>,
        ssh_keys: Vec<SSHKey>,
    }

    pub type IPAUserName = String;
    pub type LFUserName = String;

    pub struct AddRemove<T> {
        add: Vec<T>,
        remove: Vec<T>,
    }

    pub type Update<T> = Option<T>;

    pub struct UserInfoUpdateBlob {
        email: AddRemove<String>,
        ssh_keys: AddRemove<SSHKey>,
        first_name: Update<String>,
        last_name: Update<String>,
        vpn_password: Update<String>,
    }

    pub async fn update_user(username: IPAUserName, info: UserInfoUpdateBlob) {}
}
