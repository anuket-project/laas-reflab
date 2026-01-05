use std::{collections::HashMap, sync::Arc};

use anyhow::{anyhow, Result};
use common::prelude::{dashmap::DashMap, lazy_static, parking_lot, tracing};
use itertools::Itertools;
use lazy_static::lazy_static;
use models::inventory::{HostPort, Switch};

use crate::configure_networking::{BondGroup, NetworkConfig, VlanConnection};
use dal::{new_client, AsEasyTransaction, EasyTransaction, FKey};

/// valid vlan id constants per IEEE 802.1Q
pub mod constants {
    pub const MIN_VLAN_ID: i16 = 1;
    pub const MAX_VLAN_ID: i16 = 4094;
    pub const DEFAULT_VLAN_ID: i16 = 1;
}

/// Error variants for NXOS commands
#[derive(Debug, thiserror::Error)]
pub enum NXCommandError {
    #[error("Network failure connecting to {url}: {source}")]
    NetworkFailure {
        url: String,
        #[source]
        source: Box<ureq::Error>,
    },

    #[error("Switch returned HTTP error {status}: {body}")]
    HttpError { status: u16, body: String },

    #[error("Failed to parse switch response: {0}")]
    ResponseParseError(String),

    #[error("Switch returned error code {code}: {message}")]
    SwitchError { code: String, message: String },

    #[error("No commands provided to execute")]
    NoCommandsProvided,

    #[error("Lock acquisition failed for switch {0}")]
    LockError(String),
}

/// Error variants for Vlan Construction
#[derive(Debug, thiserror::Error)]
pub enum VlanError {
    #[error("Invalid VLAN ID {id}: {reason}")]
    InvalidVlanId { id: i16, reason: String },
}

/// Error variants for Network Task execution
#[derive(Debug, thiserror::Error)]
pub enum NetworkTaskError {
    #[error("Database error: {0}")]
    DatabaseError(#[from] anyhow::Error),

    #[error("Switch command failed: {0}")]
    CommandError(#[from] NXCommandError),

    #[error("Multiple switches failed: {0} failures")]
    MultipleFailures(usize),
}

/// VLAN ID 1-4094
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VlanId(i16);

impl VlanId {
    /// Create a new VLAN ID
    #[must_use = "VlanId::new returns a Result that must be handled"]
    pub fn new(id: i16) -> Result<Self, VlanError> {
        if !(constants::MIN_VLAN_ID..=constants::MAX_VLAN_ID).contains(&id) {
            return Err(VlanError::InvalidVlanId {
                id,
                reason: format!(
                    "VLAN ID must be between {} and {}",
                    constants::MIN_VLAN_ID,
                    constants::MAX_VLAN_ID
                ),
            });
        }
        Ok(VlanId(id))
    }

    /// Create a VLAN ID without validation
    ///
    /// # Safety
    /// Caller must ensure the ID is valid (1-4094)
    pub(crate) fn new_unchecked(id: i16) -> Self {
        debug_assert!(
            (constants::MIN_VLAN_ID..=constants::MAX_VLAN_ID).contains(&id),
            "VLAN ID {} is out of range",
            id
        );
        VlanId(id)
    }

    /// Get inner VLAN ID value
    pub fn get(&self) -> i16 {
        self.0
    }
}

impl std::fmt::Display for VlanId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone)]
pub struct NXCommand {
    inputs: Vec<String>,

    url: String,

    user: String,
    password: String,
}

impl std::fmt::Debug for NXCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NXCommand")
            .field("inputs", &self.inputs)
            .field("url", &self.url)
            .field("user", &"<redacted>")
            .field("password", &"<redacted>")
            .finish()
    }
}

pub struct NXCommandWithoutAuth {
    url: String,
}

impl NXCommandWithoutAuth {
    pub fn with_credentials(self, username: String, password: String) -> NXCommand {
        NXCommand {
            inputs: vec![],
            url: self.url,
            user: username,
            password,
        }
    }
}

impl NXCommand {
    pub fn for_switch(dn: String) -> NXCommandWithoutAuth {
        NXCommandWithoutAuth {
            url: format!("http://{dn}/ins"),
        }
    }

    pub fn and_then<S>(mut self, command: S) -> Self
    where
        S: Into<String>,
    {
        self.inputs.push(command.into());
        self
    }

    /// Execute NX-OS commands against a switch
    ///
    /// # Errors
    /// Returns [`NXCommandError`] if:
    /// - network communication fails
    /// - switch returns non-200 status code
    /// - switch returns an error in the response
    #[must_use = "execute() returns a Result that must be handled"]
    pub fn execute(self) -> Result<String, NXCommandError> {
        if self.inputs.is_empty() {
            return Err(NXCommandError::NoCommandsProvided);
        }

        tracing::info!("Getting switch lock");
        let lock = SWITCH_LOCK
            .entry(self.url.clone())
            .or_insert_with(|| Arc::new(parking_lot::Mutex::new(())))
            .value()
            .clone();
        tracing::info!("Got lock, now waiting for exclusive access to {}", self.url);

        tracing::info!(
            "Here is every single nx command being run on the switch: {:#?}",
            self.inputs
        );

        let _guard = lock.lock();

        let concat_input = Itertools::intersperse(self.inputs.into_iter(), " ; ".to_string())
            .reduce(|acc, e| acc + e.as_str());

        let j = ureq::json!({
            "ins_api": {
                "version": "1.0",
                "type": "cli_conf",
                "chunk": "0",
                "sid": "1",
                "output_format": "json",
                "input": concat_input,
            }
        });

        let basic_auth_header_v = format!("{}:{}", self.user, self.password);

        #[allow(deprecated)]
        let basic_auth_header = format!("Basic {}", base64::encode(basic_auth_header_v.as_bytes()));

        let resp = ureq::post(&self.url)
            .set("Authorization", &basic_auth_header)
            .set("content-type", "text/json")
            .send_json(j)
            .map_err(|e| NXCommandError::NetworkFailure {
                url: self.url.clone(),
                source: Box::new(e),
            })?;

        let status = resp.status();
        tracing::info!("Released exclusive access to switch {}", self.url);

        let body = resp
            .into_string()
            .map_err(|e| NXCommandError::ResponseParseError(e.to_string()))?;

        if status != 200 {
            return Err(NXCommandError::HttpError { status, body });
        }

        tracing::info!("Switch returned status {}, body: {:#?}", status, body);

        Ok(body)
    }
}

lazy_static! {
    static ref SWITCH_LOCK: DashMap<String, Arc<common::prelude::parking_lot::Mutex<()>>> =
        DashMap::new();
}

/// Represents the VLAN state of a switch port.
/// This type is serialized into NX-OS commands to run against a switch to configure a switch port.
#[derive(Debug, Clone, PartialEq)]
pub enum SwitchPortVlanState {
    Disabled,
    Tagged(Vec<VlanId>),
    Native(VlanId),
    TaggedAndNative {
        allowed_vlans: Vec<VlanId>,
        native_vlan: VlanId,
    },
}

impl SwitchPortVlanState {
    /// VLAN state into `Vec<String>` that represent NX-OS commands.
    pub fn to_nx_commands(&self) -> Vec<String> {
        match self {
            // Disabled state
            SwitchPortVlanState::Disabled => vec!["shutdown".to_string()],
            // Only Tagged
            SwitchPortVlanState::Tagged(allowed_vlans) => vec![
                "switchport mode trunk".to_string(),
                // disable native vlan
                "no switchport trunk native vlan".to_string(),
                format!(
                    "switchport trunk allowed vlan {}",
                    allowed_vlans_to_string(allowed_vlans)
                ),
                "no shutdown".to_string(),
            ],
            // Only Native
            SwitchPortVlanState::Native(native_vlan) => vec![
                "switchport mode trunk".to_string(),
                format!("switchport trunk native vlan {}", native_vlan.get()),
                // If Native VLAN is set, it must also be explicitly allowed
                format!("switchport trunk allowed vlan {}", native_vlan.get()),
                "no shutdown".to_string(),
            ],
            SwitchPortVlanState::TaggedAndNative {
                allowed_vlans,
                native_vlan,
            } => {
                let mut all_vlans = allowed_vlans.clone();
                all_vlans.push(*native_vlan);
                vec![
                    "switchport mode trunk".to_string(),
                    // If Native VLAN is set, it must also be explicitly allowed
                    format!(
                        "switchport trunk allowed vlan {}",
                        allowed_vlans_to_string(&all_vlans)
                    ),
                    format!("switchport trunk native vlan {}", native_vlan.get()),
                    "no shutdown".to_string(),
                ]
            }
        }
    }
}

pub async fn nx_run_network_task(network_config: NetworkConfig) -> Result<(), NetworkTaskError> {
    let mut client = new_client().await?; // Auto-converts via #[from]
    let mut transaction = client.easy_transaction().await?;

    // Process each bond group
    let switches = process_bondgroups(&network_config, &mut transaction).await?;

    // Apply commands to switches
    apply_commands_to_switches(switches, network_config.persist).await?;

    // Commit transaction
    transaction.commit().await?;

    Ok(())
}

async fn process_bondgroups(
    network_config: &NetworkConfig,
    transaction: &mut EasyTransaction<'_>,
) -> Result<HashMap<FKey<Switch>, NXCommand>> {
    let mut switches = HashMap::new();
    for bondgroup in network_config.bondgroups.clone() {
        process_bondgroup(&bondgroup, transaction, &mut switches).await?;
    }
    Ok(switches)
}

async fn process_bondgroup(
    bondgroup: &BondGroup,
    transaction: &mut EasyTransaction<'_>,
    switches: &mut HashMap<FKey<Switch>, NXCommand>,
) -> Result<()> {
    let mut for_switch = None;

    for member in bondgroup.member_host_ports.iter() {
        let switch = validate_hostport(member, transaction).await?;

        // update `for_switch`
        for_switch = match for_switch {
            None => Some(switch),
            Some(prior) => {
                // ensure that all switches are the same
                if prior != switch {
                    return Err(anyhow!("All switches must be the same for a bond group"));
                }
                Some(prior)
            }
        };
    }

    if let Some(for_switch) = for_switch {
        let for_switch = for_switch.get(transaction).await?.into_inner();

        let nxcommand = switches.remove(&for_switch.id).unwrap_or_else(|| {
            NXCommand::for_switch(for_switch.ip.clone())
                .with_credentials(for_switch.user.clone(), for_switch.pass.clone())
        });

        let nxcommand = process_ports(bondgroup, transaction, nxcommand).await?;

        switches.insert(for_switch.id, nxcommand);
    }

    Ok(())
}

async fn validate_hostport(
    member: &FKey<HostPort>,
    transaction: &mut EasyTransaction<'_>,
) -> Result<FKey<Switch>, anyhow::Error> {
    // fetch the `HostPort`
    let host_port = member
        .get(transaction)
        .await
        .map_err(|e| anyhow!("Failed to get HostPort: {}", e))?;

    // fetch the associated `SwitchPort` from the host port
    let switch_port = host_port
        .switchport
        .unwrap_or_else(|| {
            panic!(
                "HostPort {} does not have an associated SwitchPort",
                host_port.name
            )
        })
        .get(transaction)
        .await
        .map_err(|e| anyhow!("Failed to get SwitchPort: {}", e))?;

    // TODO: once SwitchOS lifecycle management is fully implemented by the import CLI. This
    // validation logic can be reimplemented. There is no way to create a new switch with a
    // specified OS and this logic could lead to failures in provisions for any new switches.

    // // fetch the associated `Switch` from the switch port
    // let switch = switch_port
    //     .for_switch
    //     .get(transaction)
    //     .await
    //     .map_err(|e| anyhow!("Failed to get Switch: {}", e))?;

    // fetch the associated `SwitchOS` from the swictch
    // let switch_os = switch
    //     .switch_os
    //     .ok_or_else(|| anyhow!("SwitchOS is not set for the Switch"))?
    //     .get(transaction)
    //     .await
    //     .map_err(|e| anyhow!("Failed to get SwitchOS: {}", e))?;

    // check if the OS type is "NXOS"
    // if switch_os.os_type != "NXOS" {
    // return Err(anyhow!("Switch OS type is not NXOS"));
    // }

    Ok(switch_port.for_switch)
}

async fn process_ports(
    bondgroup: &BondGroup,
    transaction: &mut EasyTransaction<'_>,
    mut nxcommand: NXCommand,
) -> Result<NXCommand> {
    tracing::warn!(
        "not supporting/doing actual bond groups yet, just assume each port is in a separate one"
    );

    for port in bondgroup.member_host_ports.iter() {
        nxcommand = configure_port(port, transaction, nxcommand, &bondgroup.vlans).await?;
    }

    Ok(nxcommand)
}

async fn configure_port(
    port: &FKey<HostPort>,
    transaction: &mut EasyTransaction<'_>,
    mut nxcommand: NXCommand,
    vlans: &[VlanConnection],
) -> Result<NXCommand> {
    let host_port = port.get(transaction).await?;

    let switchport = host_port
        .switchport
        .ok_or_else(|| anyhow!("HostPort does not have an associated SwitchPort"))?
        .get(transaction)
        .await?;

    nxcommand = nxcommand.and_then(format!("interface {}", switchport.name));

    let vlan_state = collect_vlan_info(vlans, transaction).await?;

    let commands = vlan_state.to_nx_commands();
    for command in commands {
        nxcommand = nxcommand.and_then(command);
    }

    tracing::info!(
        "Configured switchport {}: {:#?}",
        switchport.name,
        vlan_state
    );

    Ok(nxcommand)
}

async fn collect_vlan_info(
    vlans: &[VlanConnection],
    transaction: &mut EasyTransaction<'_>,
) -> Result<SwitchPortVlanState> {
    let mut native_vlan = None;
    let mut allowed_vlans = Vec::new();

    for vlan_connection in vlans {
        let vlan = vlan_connection.vlan.get(transaction).await?;

        // construct VlanId newtype from database value
        let vlan_id = VlanId::new(vlan.vlan_id)?;

        if !vlan_connection.tagged {
            if native_vlan.replace(vlan_id).is_some() {
                return Err(anyhow!(
                    "Multiple untagged VLANs found; only one native VLAN is allowed."
                ));
            }
        } else {
            allowed_vlans.push(vlan_id);
        }
    }

    allowed_vlans.sort_unstable();

    Ok(match (native_vlan, allowed_vlans.is_empty()) {
        (None, true) => SwitchPortVlanState::Disabled,
        (None, false) => SwitchPortVlanState::Tagged(allowed_vlans),
        (Some(native), true) => SwitchPortVlanState::Native(native),
        (Some(native), false) => SwitchPortVlanState::TaggedAndNative {
            allowed_vlans,
            native_vlan: native,
        },
    })
}

async fn apply_commands_to_switches(
    switches: HashMap<FKey<Switch>, NXCommand>,
    persist: bool,
) -> Result<(), NetworkTaskError> {
    let mut errors = Vec::new();

    for (switch_id, mut nxcommand) in switches {
        if persist {
            // save the running config to the startup config on the switch
            nxcommand = nxcommand.and_then("copy run start");
        }

        if let Err(e) = nxcommand.execute() {
            tracing::error!(
                "Failed to execute commands on switch {:?}: {}",
                switch_id,
                e
            );
            errors.push(e);
        }
    }

    if !errors.is_empty() {
        return Err(NetworkTaskError::MultipleFailures(errors.len()));
    }

    Ok(())
}

/// Helper function to format VLAN IDs as a comma-separated string.
fn allowed_vlans_to_string(allowed_vlans: &[VlanId]) -> String {
    allowed_vlans
        .iter()
        .map(|v| v.get().to_string())
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disabled_port_commands() {
        let state = SwitchPortVlanState::Disabled;
        let commands = state.to_nx_commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0], "shutdown");
    }

    #[test]
    fn test_tagged_vlans_commands() {
        let state = SwitchPortVlanState::Tagged(vec![
            VlanId::new_unchecked(100),
            VlanId::new_unchecked(200),
            VlanId::new_unchecked(300),
        ]);
        let commands = state.to_nx_commands();
        assert_eq!(commands.len(), 4);
        assert_eq!(commands[0], "switchport mode trunk");
        assert_eq!(commands[1], "no switchport trunk native vlan");
        assert_eq!(commands[2], "switchport trunk allowed vlan 100,200,300");
        assert_eq!(commands[3], "no shutdown");
    }

    #[test]
    fn test_tagged_vlans_single_vlan() {
        let state = SwitchPortVlanState::Tagged(vec![VlanId::new_unchecked(100)]);
        let commands = state.to_nx_commands();
        assert_eq!(commands[2], "switchport trunk allowed vlan 100");
    }

    #[test]
    fn test_native_vlan_commands() {
        let state = SwitchPortVlanState::Native(VlanId::new_unchecked(100));
        let commands = state.to_nx_commands();
        assert_eq!(commands.len(), 4);
        assert_eq!(commands[0], "switchport mode trunk");
        assert_eq!(commands[1], "switchport trunk native vlan 100");
        assert_eq!(commands[2], "switchport trunk allowed vlan 100");
        assert_eq!(commands[3], "no shutdown");
    }

    #[test]
    fn test_tagged_and_native_commands() {
        let state = SwitchPortVlanState::TaggedAndNative {
            allowed_vlans: vec![VlanId::new_unchecked(100), VlanId::new_unchecked(200)],
            native_vlan: VlanId::new_unchecked(300),
        };
        let commands = state.to_nx_commands();
        assert_eq!(commands.len(), 4);
        assert_eq!(commands[0], "switchport mode trunk");
        // native VLAN should be included in allowed VLANs
        assert_eq!(commands[1], "switchport trunk allowed vlan 100,200,300");
        assert_eq!(commands[2], "switchport trunk native vlan 300");
        assert_eq!(commands[3], "no shutdown");
    }

    #[test]
    fn test_tagged_and_native_preserves_native_in_allowed() {
        let state = SwitchPortVlanState::TaggedAndNative {
            allowed_vlans: vec![VlanId::new_unchecked(100), VlanId::new_unchecked(200)],
            native_vlan: VlanId::new_unchecked(150),
        };
        let commands = state.to_nx_commands();
        assert!(commands[1].contains("100,200,150") || commands[1].contains("100,150,200"));
    }

    #[test]
    fn test_allowed_vlans_to_string_single() {
        let result = allowed_vlans_to_string(&[VlanId::new_unchecked(100)]);
        assert_eq!(result, "100");
    }

    #[test]
    fn test_allowed_vlans_to_string_multiple() {
        let result = allowed_vlans_to_string(&[
            VlanId::new_unchecked(100),
            VlanId::new_unchecked(200),
            VlanId::new_unchecked(300),
        ]);
        assert_eq!(result, "100,200,300");
    }

    #[test]
    fn test_allowed_vlans_to_string_empty() {
        let result = allowed_vlans_to_string(&[]);
        assert_eq!(result, "");
    }

    #[test]
    fn test_allowed_vlans_to_string_unsorted() {
        // test order preservation
        let result = allowed_vlans_to_string(&[
            VlanId::new_unchecked(300),
            VlanId::new_unchecked(100),
            VlanId::new_unchecked(200),
        ]);
        assert_eq!(result, "300,100,200");
    }

    #[test]
    fn test_vlan_id_validation() {
        assert!(VlanId::new(1).is_ok());
        assert!(VlanId::new(100).is_ok());
        assert!(VlanId::new(4094).is_ok());

        assert!(VlanId::new(0).is_err());
        assert!(VlanId::new(-1).is_err());
        assert!(VlanId::new(4095).is_err());
        assert!(VlanId::new(5000).is_err());
    }

    #[test]
    fn test_vlan_id_display() {
        let vlan = VlanId::new_unchecked(100);
        assert_eq!(format!("{}", vlan), "100");
    }
}
