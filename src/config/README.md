# Overview

The `config` module handles the deserialization and serialization
of a yaml configuration file.
The goal of a centralized configuration is to enable other
labs/projects to use liblaas with minimal changes to the codebase.

## Structure and Implementation

The module defines a series of Rust structs that map directly to the
configuration schema expected in the `config.yaml` file.
Each struct corresponds to a specific aspect of the service, such as:

- [`LibLaaSConfig`]: The root configuration struct encapsulating
  all configuration aspects including database, web server, notifications, etc.
- [`DatabaseConfig`], [`WebConfig`], [`CliConfig`]:
  These structs represent specific
  configurations for various components of the service.

[`serde_derive`](https://serde.rs/derive.html) handles most of the deserialization for us, but some types have manual implementations. For example, [`LoggingLevel`] and [`HostPortPair`] have custom implementations.

The module provides a [`settings()`] function that returns a reference to the loaded [`LibLaaSConfig`] instance.
This function uses [`once_cell`] to ensure that the configuration is loaded exactly once. The configuration object is then accessible globally within the application. (you still have to import it)

## Usage Examples

### Accessing the Config

To access the configuration within a workspace, you can use the [`settings()`] function like so:

```rust
let config = config::settings();
println!("Database URL: {}", config.database.url.to_string());
```

### Adding New Configuration Fields

To add a new configuration field, you would first update the relevant struct.
For instance, if you wanted to add a timeout field to the
[`DatabaseConfig`], you would modify it as follows:

```rust no_run
#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    pub url: HostPortPair,
    pub username: String,
    pub password: String,
    pub database_name: String,
    pub timeout: u32, // new field added
}
```

Next, you would add this new field to the `config.yaml` file under the appropriate section:

```yaml
database:
  url: "localhost:5432"
  username: "user"
  password: "pass"
  database_name: "liblaas"
  timeout: 30 # new field value
```

[`LibLaaSConfig`]: LibLaaSConfig
[`DatabaseConfig`]: DatabaseConfig
[`WebConfig`]: WebConfig
[`CliConfig`]: CliConfig
[`LoggingLevel`]: LoggingLevel
[`HostPortPair`]: HostPortPair
[`settings()`]: settings()
[`once_cell`]: once_cell
