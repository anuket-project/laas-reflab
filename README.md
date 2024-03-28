# LibLaas

LibLaas is a service designed to manage the testing, deployment and provisioning of server instances for LaaS (Lab as a Service) testing related
to the LFEdge and Anuket projects. LibLaas was originally built to fit within the IOL's infrastructure and technology stack, but is designed
to be general purpose and should support any infrastructure as long as it is appropriately configured.

## Features

- **Web Server**: A REST API exposed to allow external services to interact with LibLaaS.
- **CLI Client**: An interactive command line interface for administrators.
- **User Management**: User management and authentication services through IPA.
- **Notifications**: User notifications supporting multiple modes ie. Email, Phone (WIP)
- **Database Access Layer**: Custom ORM designed specifically for LaaS.
- **Workflows**: Automated deployments of bookings, resource provisioning ie.
  Networks, Switches etc, IPMI configuration and other utilities.

## Workspaces

> :warning: There is not currently a way to support markdown links that work in both rustdoc and the bitbucket repo.
> For now, I will use the rustdoc supported links to avoid maintaining two different copies of each workspace's README.
> If you encounter a "broken link" this is why.

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

## Usage

Everything should be accessible through the makefile in the root of the repository.
The following commands are available:

> _"We have a gross abuse of makefiles here at the IOL" ~ Justin_

```sh
make build # build the `laas-reflab` rust binary inside a container

make up # runs docker-compose up with the built container

make cli # starts a interactive CLI client session

make stop # stops your running docker-compose containers

make edit-config # opens the configuration file in the default editor

make db-shell # opens a `psql` shell in the database container
```

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
[`laas-reflab`]: ../laas_reflab
[`liblaas`]: ../liblaas
