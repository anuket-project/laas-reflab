# LaaS Reference Lab

[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/anuket-project/laas-reflab/badge)](https://scorecard.dev/viewer/?uri=github.com/anuket-project/laas-reflab)

The LaaS Reference Lab (`laas-reflab`) project is the backend implementation for the Lab as a Service dashboard.
It exposes a REST API for resource provisioning and integrates with external services to grant access to provisioned resources.

## Features

- **Web Server**: A REST API exposed to allow external services to interact with LibLaaS.
- **CLI Client**: An interactive command line interface for administrators.
- **User Management**: User management and authentication services through IPA.
- **Notifications**: User notifications supporting multiple modes ie. Email, Phone (WIP)
- **Workflows**: Automated deployments of bookings, resource provisioning ie.
  Networks, Switches etc, IPMI configuration and other utilities.

## Project Organization

The project is primarily organized into the following workspaces (rust crates).

| Workspace         | Description                                                                                         |
| ----------------- | --------------------------------------------------------------------------------------------------- |
| [`client`]        | The CLI client that allows administrators to interact with Liblaas services.                        |
| [`common`]        | A collection of common, reexported dependencies used across different parts of the project.         |
| [`config`]        | YAML configuration parsing and deserialization                                                      |
| [`dal`]           | Database Access Layer, exposes utilities for database operations within Liblaas.                    |
| [`liblaas`]       | Axum Web API, exposes various endpoints necessary to build a frontend service consuming Liblaas     |
| [`models`]        | Database models such as `Hosts`, `Instances`, `Bookings` etc.                                       |
| [`notifications`] | Automated notifications for users and administrators.                                               |
| [`tascii`]        | we don't talk about tascii. This should tell you everything you need to know.                       |
| [`users`]         | IPA user management, oauth, automated vpn config issuing etc.                                       |
| [`workflows`]     | General purpose workspace for any task that runs externally. Miscellaneous functions and utilities. |

For more information on each workspace, please refer to the respective module's documentation.

## CI

The bamboo CI plan is defined in `bamboo.yml` file in `bamboo-specs`. This directory also has the CI/dev docker image. At this point in time the bamboo plan
has two stages that are run on every branch on every commit:

**Run Checks:**

- `check` - validate syntax
- `format` - check formatting according to the [rust-fmt style guide](https://doc.rust-lang.org/nightly/style-guide/)
- `clippy` - check for common mistakes and code style issues
- `audit` - scan for vulnerabilities in dependencies
- `machete` - mark unused dependencies
- `test` - run unit tests

**Build:**

- `build` - builds the docker image

Environment variables like CI_IMAGE and REGISTRY are injected into each job and defined in the specs file. All jobs require Docker and linuxos set on the bamboo agent.

Sometimes a run might fail for unpredictable reasons. In that case, you can re-run just the failed job(s) after the whole plan has completed.

## Using the CI Container Locally

To mirror CI locally:

1. Build the docker image:

   ```bash
   docker build -t laas-reflab-ci:latest -f bamboo-specs/Dockerfile .
   ```

2. Launch an interactive shell in the container:

```bash
docker run --rm -it -v "$(pwd)":/app -w /app laas-reflab-ci:latest bash
```

Then you can run any cargo or cargo make command just as CI would.

```bash
cargo make # builds, runs tests and outputs code coverage artifact
```

## Using Cargo Make

[`cargo-make`](https://github.com/sagiegurari/cargo-make) is leveraged to unify the dev and CI workflows.

| Task                | Profile       | Description                                                         | Dependencies                  |
|---------------------|---------------|---------------------------------------------------------------------|-------------------------------|
| **setup-db**        | development   | Start a local PostgreSQL container                                  | —                             |
| **wait-db**         | development   | Wait until the PostgreSQL container is ready                        | setup-db                      |
| **migrate**         | development   | Run SQLx migrations                                                  | setup-db, wait-db             |
| **prepare**         | development   | Prepare SQLx offline query data                                      | migrate                       |
| **test-local**      | development   | Run all tests with the local database using `cargo-nextest`          | prepare                       |
| **test-local-coverage** | development | Generate LCOV coverage report (`.coverage/lcov.info`)               | prepare, install-llvm-cov     |
| **ci**              | ci            | Full CI pipeline entry point; runs tests and emits HTML coverage     | prepare, install-llvm-cov     |
| **fmt**             | all           | Check code formatting (`cargo fmt --all --check`)                   | —                             |
| **clippy**          | all           | Run strict lint checks (`cargo lclippy --all-targets … -D warnings`)| —                             |
| **audit**           | all           | Security vulnerability scan (`cargo audit`)                          | —                             |
| **machete**         | all           | Detect unused dependencies (`cargo machete`)                         | —                             |
| **check**           | all           | Dependency graph & SQLx offline validation (`cargo lcheck …`)        | —                             |

## SQLx Offline Mode and the `.sqlx` Directory

To compile [`sqlx`](https://github.com/launchbadge/sqlx) macros without a live database, we use a feature called "offline mode". At workspace root, a `.sqlx` folder holds JSON metadata for every sqlx `query!` invocation. This directory **must** be committed so CI can validate queries against the DB schema during the build step.

If you use `cargo make` the `.sqlx` directory will automatically be updated, however if you need to do it manually, you can run:

> WARNING: Requires $DATABASE_URL to be set to a valid postgres connection string with all migrations from `migrations/`applied.

```
cargo sqlx prepare --workspace
```

## Code Coverage

`cargo-llvm-cov` is used to generate code coverage reports. The coverage artifact should be output as `.coverage/lcov.info` after running `cargo-make` and can be loaded into your IDE of choice from there.

If you would like to generate an html report, you can run:

```bash
cargo llvm-cov --workspace --html nextest
```

The `.html` file will be spit out at `.coverage/llvm-cov/html/index.html` and can be opened in your browser.

## Common Issues

- Do not forget to export $DATABASE_URL before running any commands that require a database connection.
- Always include the `.sqlx` directory in your commits.

[`client`]: ../client
[`common`]: ../common
[`config`]: ../config
[`dal`]: ../dal
[`liblaas`]: ../liblaas
[`models`]: ../models
[`notifications`]: ../notifications
[`tascii`]: ../tascii
[`users`]: ../users
[`workflows`]: ../workflows
