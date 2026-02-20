use models::inventory::HostPort;
use notifications::templates::render_template;
use tascii::prelude::Uuid;
use users::ipa;

use crate::{
    configure_networking::vlan_connection::NetworkManagerVlanConnection,
    resource_management::mailbox::Endpoint, EthernetConnectionFormatted, IpaUserFormatted,
    VlanConnectionFormatted,
};

pub fn render_vendor_data(
    ipa_users: Vec<ipa::User>,
    hostname: String,
    post_provision_endpoint: Endpoint,
) -> Result<String, tera::Error> {
    let mut template_context = tera::Context::new();

    template_context.insert("ipa_users", &IpaUserFormatted::from_users(ipa_users));
    template_context.insert("hostname", &hostname);
    template_context.insert("mailbox_endpoint", &post_provision_endpoint.to_url());

    render_template("generic/cloud-init/vendor-data.j2", &template_context)
}

pub fn render_network_config(
    ports: Vec<HostPort>,
    nm_connections: Vec<NetworkManagerVlanConnection>,
) -> Result<String, tera::Error> {
    let mut template_context = tera::Context::new();

    template_context.insert(
        "ethernet_interfaces",
        &EthernetConnectionFormatted::from_ports(ports),
    );
    template_context.insert(
        "vlans",
        &VlanConnectionFormatted::from_nm_connections(nm_connections),
    );

    render_template("generic/cloud-init/network-config.j2", &template_context)
}

// Here to make expansion of meta-data easier in the future if needed
//      despite meta-data being really small at the moment
pub fn render_meta_data(instance_id: Uuid) -> Result<String, tera::Error> {
    let mut template_context = tera::Context::new();

    template_context.insert("instance_id", &instance_id);

    render_template("generic/cloud-init/meta-data.j2", &template_context)
}
