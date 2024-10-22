# Overview

The [`liblaas`] crate contains the web API for [`laas-reflab`]. [`liblaas`]
is built on top of [`axum`]. If you are familiar with other web frameworks
within the Rust ecosystem, [`axum`] is also built on top of [`hyper`]
and [`tower`] by the creators of [`tokio`]. A full guide on [`axum`] is out
of the scope of this documentation, but you should be able
to pick it up in weekend or so if you are familiar with Rust.

## Structure

> ⚠️ **Note**: This specific structure is likely to change, but the general categories should still apply. ie. `web` contains
> all the routes and handlers with nested folders for each _thing_ that the API does.

### Files and Folders

```plaintext
 src
 ├─ booking
 │  ├─ README.md
 │  └─ mod.rs // booking utils (no routes)
 ├─ lib.rs // you are here
 └─ web // all routes and handlers go in '/web'
    ├─ api.rs // web safe views of models called 'blobs'
    ├─ apispec.yaml // openapi spec
    ├─ booking
    │  └─ mod.rs // booking routes
    ├─ docs.rs // auto generated documentation
    ├─ flavor
    │  └─ mod.rs // flavor routes
    ├─ mod.rs // API construction and route registration
    ├─ template
    │  └─ mod.rs // template routes
    └─ users
      └─ mod.rs // user routes
```

### Endpoints

- **`/booking`**: Requests related to booking and provisioning servers.
- **`/user`**: Everything user related, SSH key management, user profiles, [`IPA`] integration etc.
- **`/template`**: Templates represent a collection of configurations and flavors
  that the user can choose from when booking a server.
- **`/flavor`**: Flavors are the hardware configurations that the user can choose from. ie.
  `HPE x86 Gen9 25G` when making templates/bookings.
- **`/docs`**: Auto generated documentation for the API using [`aide`]

## Implementation

### Endpoints

[`liblaas`] is organized by defining routes and their corresponding
handlers within a folder in `/web`, such as `web/booking/mod.rs` for the `/booking`
endpoint.

```rust

// variable definitions for state, api, api_docs etc.

 let app = ApiRouter::new()
    .nest_api_service("/booking", booking::routes(state.clone()))
    .nest_api_service("/flavor", flavor::routes(state.clone()))
    .nest_api_service("/template", template::routes(state.clone()))
    .nest_api_service("/user", users::routes(state.clone()))
    .nest_api_service("/docs", docs_routes(state.clone()))
    .finish_api_with(&mut api, api_docs)
    .layer(Extension(Arc::new(api)))
    .with_state(state);
```

This snippet builds and registers each API `service` defined in
each modules and nests them under their respective endpoint prefixes. Think of a
`service` as a collection or bundle of routes and handlers.

### State

The following snippet is a construed (don't ask me why this actually exists in `mod.rs` because I don't know.)
example of a custom struct that can be passed into [`axum`] services for handlers to pick up with extractors.

```rust
 #[derive(Debug, Clone, Default)]
 pub struct AppState {
   state: String,
 }
```

[`AppState`] can then be injected into the application through `.with_state(state)`
and can be accessed by providing an extractor in the argument of a handler like so:

```rust
async fn handler(
    // access the state via the `State` extractor
    // extracting a state of the wrong type results in a compile error
    State(state): State<AppState>,
) {
    // use `state`...
}
```

### Documentation

[`aide`] is a crate that automatically generates documentation with support for several
web frameworks including [`axum`]. The following shows how we use [`OpenApi`]'s
[`default`](std::default::Default) Implementation to initalize an instance of the struct and configure it.

```rust
  let mut api = OpenApi::default();
  // Configuration for OpenAPI
  let api = OpenApi {
    info: Info {
    description: Some("Booking API".to_string()),
    ..Info::default()
  },
  // more configuration
};
```

This [`OpenApi`] instance is then used by `.finish_api_with(&mut api, api_docs)`, which will serve
a live version of the generated documentation at `/docs`.

### Serving the API

Finally, [`liblaas`] binds the [`axum`] HTTP application to a TCP port specified in the [`config`] file.

```rust
 let api_addr = config::settings().web.bind_addr.to_string();

 tracing::info!("Binding to {}", api_addr);

 let res = axum::Server::bind(
    &std::net::SocketAddr::from_str(&api_addr).expect("Expected api address as a string."),
    )
    .serve(app.into_make_service())
    .await;
```

[`aide`]: aide
[`axum`]: axum
[`hyper`]: https://docs.rs/hyper
[`tower`]: https://docs.rs/tower
[`tokio`]: https://docs.rs/tokio
[`liblaas`]: self
[`laas-reflab`]: ../laas_reflab/index.html
[`IPA`]: ../users/ipa/index.html
[`OpenApi`]: common::prelude::aide::openapi::OpenApi
[`AppState`]: web::AppState
[`config`]: ../config/index.html
