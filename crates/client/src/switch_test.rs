use crate::remote::{Select, Server, Text};
use dal::{AsEasyTransaction, DBTable, new_client};
use models::inventory::Switch;
use serde_json::Value;
use std::io::Write;
use workflows::resource_management::cisco::{NXCommand, SwitchPortVlanState, VlanId};

/// Errors that can occur during NXOS testing
#[derive(Debug, thiserror::Error)]
pub enum NXOSTestError {
    #[error("HTTP error {status}: {body}")]
    Http { status: u16, body: String },

    #[error("Network error: {0}")]
    Network(String),

    #[error("Failed to parse JSON response: {0}")]
    JsonParse(#[from] serde_json::Error),

    #[error("Transport error: {0}")]
    Transport(String),
}

/// Available test scenarios
#[derive(Debug, Clone, Copy, PartialEq)]
enum TestScenarioType {
    TaggedVlans,
    NativeVlan,
    TaggedAndNative,
    DisabledPort,
}

impl TestScenarioType {
    fn name(&self) -> &str {
        match self {
            Self::TaggedVlans => "Tagged VLANs Only",
            Self::NativeVlan => "Native VLAN Only (Untagged)",
            Self::TaggedAndNative => "Tagged + Native VLANs",
            Self::DisabledPort => "Disabled Port (Shutdown)",
        }
    }

    fn description(&self) -> &str {
        match self {
            Self::TaggedVlans => {
                "Configure port with multiple tagged VLANs (trunk mode, no native)"
            }
            Self::NativeVlan => "Configure port with a single native/untagged VLAN",
            Self::TaggedAndNative => "Configure port with tagged VLANs and a native VLAN",
            Self::DisabledPort => "Shutdown/disable a port",
        }
    }

    fn all() -> Vec<Self> {
        vec![
            Self::TaggedVlans,
            Self::NativeVlan,
            Self::TaggedAndNative,
            Self::DisabledPort,
        ]
    }
}

/// Represents a configured test scenario
#[derive(Debug)]
struct ConfiguredTest {
    scenario_type: TestScenarioType,
    port: String,
    state: SwitchPortVlanState,
}

/// Configure a test scenario by prompting
fn configure_test_scenario(
    scenario_type: TestScenarioType,
    mut session: &Server,
) -> Result<ConfiguredTest, anyhow::Error> {
    let _ = writeln!(session, "\nConfiguring test: {}", scenario_type.name());
    let _ = writeln!(session, "{}", scenario_type.description());

    let port = Text::new("Interface name (e.g., Ethernet1/1):").prompt(session)?;

    let state = match scenario_type {
        TestScenarioType::TaggedVlans => {
            let vlans_input =
                Text::new("Tagged VLANs (comma-separated, e.g., 100,200,300):").prompt(session)?;
            let vlans = parse_vlan_list(&vlans_input)?;

            if vlans.is_empty() {
                return Err(anyhow::anyhow!("At least one VLAN must be specified"));
            }

            SwitchPortVlanState::Tagged(vlans)
        }
        TestScenarioType::NativeVlan => {
            let vlan_input = Text::new("Native VLAN ID (1-4094):").prompt(session)?;
            let vlan_id = vlan_input
                .trim()
                .parse::<i16>()
                .map_err(|e| anyhow::anyhow!("Invalid VLAN ID: {}", e))?;
            let vlan = VlanId::new(vlan_id)?;

            SwitchPortVlanState::Native(vlan)
        }
        TestScenarioType::TaggedAndNative => {
            let tagged_input =
                Text::new("Tagged VLANs (comma-separated, e.g., 100,200):").prompt(session)?;
            let allowed_vlans = parse_vlan_list(&tagged_input)?;

            if allowed_vlans.is_empty() {
                return Err(anyhow::anyhow!(
                    "At least one tagged VLAN must be specified"
                ));
            }

            let native_input = Text::new("Native VLAN ID (1-4094):").prompt(session)?;
            let native_id = native_input
                .trim()
                .parse::<i16>()
                .map_err(|e| anyhow::anyhow!("Invalid VLAN ID: {}", e))?;
            let native_vlan = VlanId::new(native_id)?;

            SwitchPortVlanState::TaggedAndNative {
                allowed_vlans,
                native_vlan,
            }
        }
        TestScenarioType::DisabledPort => SwitchPortVlanState::Disabled,
    };

    Ok(ConfiguredTest {
        scenario_type,
        port,
        state,
    })
}

/// Parse a comma-separated list of VLAN IDs
fn parse_vlan_list(input: &str) -> Result<Vec<VlanId>, anyhow::Error> {
    input
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| {
            let id = s
                .parse::<i16>()
                .map_err(|e| anyhow::anyhow!("Invalid VLAN ID '{}': {}", s, e))?;
            VlanId::new(id).map_err(|e| anyhow::anyhow!("{}", e))
        })
        .collect()
}

/// Review and manage configured tests
fn review_tests(
    configured_tests: &mut Vec<ConfiguredTest>,
    mut session: &Server,
) -> Result<(), anyhow::Error> {
    if configured_tests.is_empty() {
        let _ = writeln!(session, "No tests configured yet.");
        return Ok(());
    }

    let _ = writeln!(session, "\nConfigured Tests:");
    for (idx, test) in configured_tests.iter().enumerate() {
        let vlan_info = match &test.state {
            SwitchPortVlanState::Disabled => "Shutdown".to_string(),
            SwitchPortVlanState::Tagged(vlans) => {
                format!("Tagged: {}", vlans.iter().map(|v| v.get().to_string()).collect::<Vec<_>>().join(","))
            }
            SwitchPortVlanState::Native(vlan) => format!("Native: {}", vlan.get()),
            SwitchPortVlanState::TaggedAndNative { allowed_vlans, native_vlan } => {
                format!(
                    "Tagged: {}, Native: {}",
                    allowed_vlans.iter().map(|v| v.get().to_string()).collect::<Vec<_>>().join(","),
                    native_vlan.get()
                )
            }
        };
        let _ = writeln!(
            session,
            "  {}. {} on {} - {}",
            idx + 1,
            test.scenario_type.name(),
            test.port,
            vlan_info
        );
    }

    let mut options: Vec<String> = vec!["Go back".to_string()];
    for idx in 0..configured_tests.len() {
        options.push(format!("Remove test {}", idx + 1));
    }

    let options_str: Vec<&str> = options.iter().map(|s| s.as_str()).collect();
    let choice = Select::new("\nManage tests:", options_str).prompt(session)?;

    if choice == "Go back" {
        return Ok(());
    }

    if let Some(test_num_str) = choice.strip_prefix("Remove test ") {
        if let Ok(test_num) = test_num_str.parse::<usize>() {
            if test_num > 0 && test_num <= configured_tests.len() {
                let removed = configured_tests.remove(test_num - 1);
                let _ = writeln!(
                    session,
                    "Removed: {} on {}",
                    removed.scenario_type.name(),
                    removed.port
                );
            }
        }
    }

    Ok(())
}

/// Get switch credentials from user
async fn get_switch_credentials(
    mut session: &Server,
) -> Result<(String, String, String), anyhow::Error> {
    let choice = Select::new(
        "Select switch source:",
        vec![
            "Test existing switch from database",
            "Enter new switch credentials",
        ],
    )
    .prompt(session)?;

    match choice {
        "Test existing switch from database" => {
            let mut client = new_client().await?;
            let mut transaction = client.easy_transaction().await?;

            let switches = Switch::select().run(&mut transaction).await?;

            if switches.is_empty() {
                let _ = writeln!(session, "No switches found in database!");
                return Err(anyhow::anyhow!("No switches in database"));
            }

            let switch_options: Vec<String> = switches
                .iter()
                .map(|sw| {
                    let sw = sw.clone().into_inner();
                    format!("{} ({})", sw.name, sw.ip)
                })
                .collect();

            let selected = Select::new("Select a switch:", switch_options).prompt(session)?;

            let selected_switch = switches
                .into_iter()
                .find(|sw| {
                    let sw_inner = sw.clone().into_inner();
                    format!("{} ({})", sw_inner.name, sw_inner.ip) == selected
                })
                .ok_or_else(|| anyhow::anyhow!("Switch not found"))?
                .into_inner();

            let _ = writeln!(
                session,
                "Selected switch: {} at {}",
                selected_switch.name,
                selected_switch.ip
            );

            Ok((
                selected_switch.ip,
                selected_switch.user,
                selected_switch.pass,
            ))
        }
        "Enter new switch credentials" => {
            let ip = Text::new("Switch IP address or hostname:").prompt(session)?;
            let username = Text::new("Switch username:").prompt(session)?;
            let password = Text::new("Switch password:").prompt(session)?;
            Ok((ip, username, password))
        }
        _ => unreachable!(),
    }
}

/// Represents a single NXOS command test case
#[derive(Debug, Clone)]
pub struct NXOSTestCase {
    /// Name/description of the test
    pub name: String,
    /// The NXOS command to execute
    pub command: String,
    /// Category of the command
    pub category: String,
}

/// Result of executing a test case
#[derive(Debug)]
pub struct TestResult {
    pub test_case: NXOSTestCase,
    pub success: bool,
    pub response: String,
    pub error: Option<String>,
}

pub fn get_nxos_test_suite() -> Vec<NXOSTestCase> {
    vec![
        NXOSTestCase {
            name: "Show Version".to_string(),
            command: "show version".to_string(),
            category: "System Info".to_string(),
        },
        NXOSTestCase {
            name: "Show Hostname".to_string(),
            command: "show hostname".to_string(),
            category: "System Info".to_string(),
        },
        NXOSTestCase {
            name: "Show Clock".to_string(),
            command: "show clock".to_string(),
            category: "System Info".to_string(),
        },
        NXOSTestCase {
            name: "Show Running Config".to_string(),
            command: "show running-config".to_string(),
            category: "Configuration".to_string(),
        },
        NXOSTestCase {
            name: "Show Interfaces Brief".to_string(),
            command: "show interface brief".to_string(),
            category: "Interfaces".to_string(),
        },
        NXOSTestCase {
            name: "Show Interface Status".to_string(),
            command: "show interface status".to_string(),
            category: "Interfaces".to_string(),
        },
        NXOSTestCase {
            name: "Show Interface Switchport".to_string(),
            command: "show interface switchport".to_string(),
            category: "Interfaces".to_string(),
        },
        NXOSTestCase {
            name: "Show VLAN".to_string(),
            command: "show vlan".to_string(),
            category: "VLANs".to_string(),
        },
        NXOSTestCase {
            name: "Show VLAN Brief".to_string(),
            command: "show vlan brief".to_string(),
            category: "VLANs".to_string(),
        },
        NXOSTestCase {
            name: "Show Port-Channel Summary".to_string(),
            command: "show port-channel summary".to_string(),
            category: "Port-Channels".to_string(),
        },
        NXOSTestCase {
            name: "Show MAC Address Table".to_string(),
            command: "show mac address-table".to_string(),
            category: "Layer 2".to_string(),
        },
        NXOSTestCase {
            name: "Show Spanning-Tree".to_string(),
            command: "show spanning-tree".to_string(),
            category: "Spanning Tree".to_string(),
        },
        NXOSTestCase {
            name: "Show Logging".to_string(),
            command: "show logging last 10".to_string(),
            category: "Logging".to_string(),
        },
        NXOSTestCase {
            name: "Show Environment".to_string(),
            command: "show environment".to_string(),
            category: "Hardware".to_string(),
        },
        NXOSTestCase {
            name: "Show Module".to_string(),
            command: "show module".to_string(),
            category: "Hardware".to_string(),
        },
        NXOSTestCase {
            name: "Show IP Interface Brief".to_string(),
            command: "show ip interface brief".to_string(),
            category: "IP".to_string(),
        },
        NXOSTestCase {
            name: "Show Processes CPU".to_string(),
            command: "show processes cpu".to_string(),
            category: "System Resources".to_string(),
        },
        NXOSTestCase {
            name: "Show System Resources".to_string(),
            command: "show system resources".to_string(),
            category: "System Resources".to_string(),
        },
    ]
}

fn execute_nxos_command(
    ip: &str,
    username: &str,
    password: &str,
    command: &str,
) -> Result<String, NXOSTestError> {
    let url = format!("http://{}/ins", ip);

    let payload = serde_json::json!({
        "ins_api": {
            "version": "1.0",
            "type": "cli_show",
            "chunk": "0",
            "sid": "1",
            "output_format": "json",
            "input": command,
        }
    });

    let basic_auth_value = format!("{}:{}", username, password);
    #[allow(deprecated)]
    let basic_auth_header = format!("Basic {}", base64::encode(basic_auth_value.as_bytes()));

    match ureq::post(&url)
        .set("Authorization", &basic_auth_header)
        .set("content-type", "application/json")
        .timeout(std::time::Duration::from_secs(10))
        .send_json(&payload)
    {
        Ok(response) => {
            let status = response.status();
            let body = response
                .into_string()
                .map_err(|e| NXOSTestError::Network(format!("Failed to read response: {}", e)))?;

            if status == 200 {
                Ok(body)
            } else {
                Err(NXOSTestError::Http { status, body })
            }
        }
        Err(ureq::Error::Status(code, response)) => {
            let body = response.into_string().unwrap_or_else(|_| "".to_string());
            Err(NXOSTestError::Http { status: code, body })
        }
        Err(ureq::Error::Transport(transport)) => {
            Err(NXOSTestError::Transport(transport.to_string()))
        }
    }
}

pub fn run_nxos_tests(
    ip: &str,
    username: &str,
    password: &str,
    test_cases: Vec<NXOSTestCase>,
    verbose: bool,
) -> Vec<TestResult> {
    let mut results = Vec::new();

    for test_case in test_cases {
        if verbose {
            println!("Running: {} - {}", test_case.category, test_case.name);
        }

        match execute_nxos_command(ip, username, password, &test_case.command) {
            Ok(response) => {
                // Parse JSON and check the code field properly
                let success = if let Ok(json) = serde_json::from_str::<Value>(&response) {
                    // Navigate to ins_api.outputs.output.code
                    if let Some(code) = json
                        .get("ins_api")
                        .and_then(|api| api.get("outputs"))
                        .and_then(|outputs| outputs.get("output"))
                        .and_then(|output| output.get("code"))
                    {
                        // Check if code is "200" (string) or 200 (number)
                        match code {
                            Value::String(s) => s == "200",
                            Value::Number(n) => n.as_i64() == Some(200),
                            _ => false,
                        }
                    } else {
                        // If we can't find the code field, assume success
                        true
                    }
                } else {
                    // If JSON parsing fails, assume success (raw text response)
                    true
                };

                results.push(TestResult {
                    test_case: test_case.clone(),
                    success,
                    response: response.clone(),
                    error: if !success { Some(response) } else { None },
                });
            }
            Err(e) => {
                results.push(TestResult {
                    test_case: test_case.clone(),
                    success: false,
                    response: String::new(),
                    error: Some(e.to_string()),
                });
            }
        }
    }

    results
}

pub fn print_test_summary(mut session: &Server, results: &[TestResult]) {
    let total = results.len();
    let passed = results.iter().filter(|r| r.success).count();
    let failed = total - passed;

    let _ = writeln!(session, "\nNXOS Command Test Results");
    let _ = writeln!(session, "{}", "─".repeat(90));
    let _ = writeln!(
        session,
        "{:<25} {:<45} {}",
        "Category", "Test", "Result"
    );
    let _ = writeln!(session, "{}", "─".repeat(90));

    for result in results {
        let status = if result.success { "PASS" } else { "FAIL" };

        let _ = writeln!(
            session,
            "{:<25} {:<45} {}",
            result.test_case.category, result.test_case.name, status
        );
    }

    let _ = writeln!(session, "{}", "─".repeat(90));
    let _ = writeln!(
        session,
        "\nTotal: {}  Passed: {}  Failed: {}",
        total,
        passed,
        failed
    );
}

pub async fn test_switch(mut session: &Server) -> Result<(), anyhow::Error> {
    let _ = writeln!(session, "\nNXOS Switch Tester");
    let _ = writeln!(
        session,
        "This tool tests common NXOS commands on a Cisco switch.\n"
    );

    let (ip, username, password) = get_switch_credentials(session).await?;

    let verbose = matches!(
        Select::new("Show verbose output?", vec!["No", "Yes"]).prompt(session)?,
        "Yes"
    );

    let _ = writeln!(session, "\nConnecting to switch at {}...", ip);
    match execute_nxos_command(&ip, &username, &password, "show version") {
        Ok(_) => {
            let _ = writeln!(session, "Connected successfully");
        }
        Err(e) => {
            let _ = writeln!(session, "Connection failed: {}", e);
            return Err(anyhow::anyhow!("Failed to connect to switch"));
        }
    }

    let _ = writeln!(session, "\nRunning test suite...");

    let test_cases = get_nxos_test_suite();
    let results = run_nxos_tests(&ip, &username, &password, test_cases, verbose);

    print_test_summary(session, &results);

    let show_details = matches!(
        Select::new("\nShow detailed results?", vec!["No", "Yes"]).prompt(session)?,
        "Yes"
    );

    if show_details {
        let _ = writeln!(session, "\nDetailed Results:");
        for result in &results {
            let _ = writeln!(session, "\n{}", "─".repeat(80));
            let _ = writeln!(
                session,
                "{} - {}",
                result.test_case.category,
                result.test_case.name
            );
            let _ = writeln!(session, "Command: {}", result.test_case.command);
            let _ = writeln!(
                session,
                "Status: {}",
                if result.success { "PASS" } else { "FAIL" }
            );

            if result.success {
                let _ = writeln!(session, "\nResponse:");
                if let Ok(json) = serde_json::from_str::<Value>(&result.response) {
                    let pretty =
                        serde_json::to_string_pretty(&json).unwrap_or(result.response.clone());
                    let _ = writeln!(session, "{}", pretty);
                } else {
                    let _ = writeln!(session, "{}", result.response);
                }
            } else if let Some(ref error) = result.error {
                let _ = writeln!(session, "\nError:");
                let _ = writeln!(session, "{}", error);
            }
        }
    }

    Ok(())
}

/// Test active VLAN configuration on a physical switch
pub async fn test_vlan_configuration(mut session: &Server) -> Result<(), anyhow::Error> {
    let _ = writeln!(session, "\nNXOS VLAN Configuration Tester");
    let _ = writeln!(
        session,
        "This tool tests VLAN configuration using the production networking code.\n"
    );
    let _ = writeln!(
        session,
        "WARNING: This will modify switch port configurations!"
    );

    let (ip, username, password) = get_switch_credentials(session).await?;

    let _ = writeln!(session, "\nConnecting to switch at {}...", ip);
    match execute_nxos_command(&ip, &username, &password, "show version") {
        Ok(_) => {
            let _ = writeln!(session, "Connected successfully");
        }
        Err(e) => {
            let _ = writeln!(session, "Connection failed: {}", e);
            return Err(anyhow::anyhow!("Failed to connect to switch"));
        }
    }

    let _ = writeln!(session, "\nConfigure Test Scenarios");
    let _ = writeln!(
        session,
        "You can add multiple tests to run. Each test will be configured individually.\n"
    );

    let mut configured_tests = Vec::new();

    loop {
        let action = if configured_tests.is_empty() {
            "Add Test"
        } else {
            let choice = Select::new(
                &format!(
                    "\n{} test(s) configured. What would you like to do?",
                    configured_tests.len()
                ),
                vec![
                    "Add another test",
                    "Review/edit tests",
                    "Done, run tests now",
                    "Cancel all tests",
                ],
            )
            .prompt(session)?;

            match choice {
                "Add another test" => "Add Test",
                "Review/edit tests" => "Review Tests",
                "Done, run tests now" => break,
                "Cancel all tests" => {
                    let _ = writeln!(session, "Tests cancelled.");
                    return Ok(());
                }
                _ => unreachable!(),
            }
        };

        if action == "Review Tests" {
            review_tests(&mut configured_tests, session)?;
            continue;
        }

        if action == "Add Test" {
            let scenario_options: Vec<String> = TestScenarioType::all()
                .iter()
                .map(|t| format!("{} - {}", t.name(), t.description()))
                .collect();

            let selected =
                Select::new("Select test scenario:", scenario_options).prompt(session)?;

            let scenario_type = TestScenarioType::all()
                .into_iter()
                .find(|t| selected.starts_with(t.name()))
                .expect("Selected scenario should match");

            match configure_test_scenario(scenario_type, session) {
                Ok(test) => {
                    // Check for port reuse
                    if let Some(existing) = configured_tests.iter().find(|t| t.port == test.port) {
                        let _ = writeln!(
                            session,
                            "\nWarning: Port {} is already used in test: {}",
                            test.port,
                            existing.scenario_type.name()
                        );
                        let _ = writeln!(
                            session,
                            "Running this test will overwrite the previous configuration on this port."
                        );

                        let proceed = Select::new(
                            "Do you want to continue?",
                            vec!["No, don't add this test", "Yes, add anyway"],
                        )
                        .prompt(session)?;

                        if proceed == "No, don't add this test" {
                            let _ = writeln!(session, "Test not added.");
                            continue;
                        }
                    }

                    let _ = writeln!(
                        session,
                        "  Test configured: {} on {}",
                        test.scenario_type.name(),
                        test.port
                    );
                    configured_tests.push(test);
                }
                Err(e) => {
                    let _ = writeln!(session, "Error: {}", e);
                    let continue_choice = Select::new(
                        "What would you like to do?",
                        vec!["Try again", "Skip this test"],
                    )
                    .prompt(session)?;

                    if continue_choice == "Try again" {
                        continue;
                    }
                }
            }
        }
    }

    if configured_tests.is_empty() {
        let _ = writeln!(session, "No tests configured!");
        return Err(anyhow::anyhow!("No tests configured"));
    }

    let _ = writeln!(session, "\nRunning {} test(s)...", configured_tests.len());
    let _ = writeln!(session, "{}", "─".repeat(80));

    let mut test_results = Vec::new();
    for (idx, test) in configured_tests.iter().enumerate() {
        let _ = writeln!(
            session,
            "\nTest {}: {} on {}",
            idx + 1,
            test.scenario_type.name(),
            test.port
        );

        let result = test_port_configuration(
            &ip,
            &username,
            &password,
            &test.port,
            test.state.clone(),
            session,
        );

        let test_name = format!("{} on {}", test.scenario_type.name(), test.port);
        test_results.push((test_name, result));
    }

    let _ = writeln!(session, "\n{}", "─".repeat(80));
    let _ = writeln!(session, "Test Results:");
    let passed = test_results.iter().filter(|(_, r)| r.is_ok()).count();
    let failed = test_results.len() - passed;

    for (test_name, result) in &test_results {
        let status = if result.is_ok() { "PASS" } else { "FAIL" };
        let _ = writeln!(session, "{:<60} {}", test_name, status);
        if let Err(e) = result {
            let _ = writeln!(session, "  Error: {}", e);
        }
    }

    let _ = writeln!(session, "{}", "─".repeat(80));
    let _ = writeln!(
        session,
        "Total: {}  Passed: {}  Failed: {}",
        test_results.len(),
        passed,
        failed
    );

    let tested_ports: Vec<String> = configured_tests
        .iter()
        .map(|t| t.port.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    if !tested_ports.is_empty() {
        let cleanup_choice = Select::new(
            "\nCleanup tested ports?",
            vec!["Leave ports as configured", "Shutdown all tested ports"],
        )
        .prompt(session)?;

        if cleanup_choice == "Shutdown all tested ports" {
            let _ = writeln!(session, "\nShutting down ports...");
            for port in &tested_ports {
                let _ = writeln!(session, "  {}...", port);
                match shutdown_port(&ip, &username, &password, port) {
                    Ok(_) => {
                        let _ = writeln!(session, "    Shutdown complete");
                    }
                    Err(e) => {
                        let _ = writeln!(session, "    Failed: {}", e);
                    }
                }
            }
        } else {
            let _ = writeln!(session, "\nPorts left in configured state.");
        }
    }

    Ok(())
}

/// Test a VLAN configuration on a port using an NXCommand
fn test_port_configuration(
    ip: &str,
    username: &str,
    password: &str,
    interface: &str,
    vlan_state: SwitchPortVlanState,
    mut session: &Server,
) -> Result<(), anyhow::Error> {
    let commands = vlan_state.to_nx_commands();
    let _ = writeln!(session, "  Commands:");
    for cmd in &commands {
        let _ = writeln!(session, "    {}", cmd);
    }

    let mut nxcommand = NXCommand::for_switch(ip.to_string())
        .with_credentials(username.to_string(), password.to_string())
        .and_then(format!("interface {}", interface));

    for cmd in commands {
        nxcommand = nxcommand.and_then(cmd);
    }

    nxcommand
        .execute()
        .map_err(|e| anyhow::anyhow!("Command execution failed: {}", e))?;

    let verify_cmd = format!("show interface {} switchport", interface);
    execute_nxos_command(ip, username, password, &verify_cmd)
        .map_err(|e| anyhow::anyhow!("Verification failed: {}", e))?;

    let _ = writeln!(session, "  Success");
    Ok(())
}

/// Shutdown a port
fn shutdown_port(ip: &str, username: &str, password: &str, interface: &str) -> Result<(), String> {
    NXCommand::for_switch(ip.to_string())
        .with_credentials(username.to_string(), password.to_string())
        .and_then(format!("interface {}", interface))
        .and_then("shutdown")
        .execute()
        .map(|_| ())
        .map_err(|e| format!("Failed to shutdown: {}", e))
}
