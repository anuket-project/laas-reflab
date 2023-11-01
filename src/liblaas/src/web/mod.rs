//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use common::prelude::*;

use aide::transform::TransformOpenApi;
use axum::{extract::Json, http::StatusCode, Extension};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use aide::{
    axum::{ApiRouter, IntoApiResponse},
    openapi::{OpenApi, Tag},
};

use aide::openapi::Info;
use tascii::prelude::Runtime;

use docs::docs_routes;
use models::dal::web::ApiError;
use std::{str::FromStr, sync::Arc};

pub mod api;
pub mod booking;
mod docs;
mod flavor;
pub mod template;
pub mod users;

pub type WebError = (StatusCode, String);

// TODO: use new error type
// TODO: make all tuples into appropriate structs

#[derive(Debug, Clone, Default)]
pub struct AppState {
    state: String,
}

#[derive(Serialize, Deserialize, Debug, JsonSchema)]
pub struct Metadata {
    user_id: Option<i64>,
}

#[derive(Serialize, Deserialize, Debug, JsonSchema)]
pub struct LLRequest<T: Serialize + std::fmt::Debug + JsonSchema> {
    pub payload: T,
    pub metadata: Metadata,
}

impl<T: Serialize + std::fmt::Debug + JsonSchema> LLRequest<T> {
    pub fn split(self) -> (T, Metadata) {
        (self.payload, self.metadata)
    }
}

pub async fn entry(rt: &'static Runtime) {
    let state = AppState::default();
    let mut api = OpenApi::default();

    async fn serve_api(Extension(api): Extension<OpenApi>) -> impl IntoApiResponse {
        Json(api)
    }

    let app = ApiRouter::new()
        .nest_api_service("/booking", booking::routes(state.clone()))
        .nest_api_service("/flavor", flavor::routes(state.clone()))
        .nest_api_service("/template", template::routes(state.clone()))
        .nest_api_service("/user", users::routes(state.clone()))
        .nest_api_service("/docs", docs_routes(state.clone()))
        .finish_api_with(&mut api, api_docs)
        .layer(Extension(Arc::new(api)))
        .with_state(state);

    let api = OpenApi {
        info: Info {
            description: Some("Booking API".to_string()),
            ..Info::default()
        },
        ..OpenApi::default()
    };

    fn api_docs(api: TransformOpenApi) -> TransformOpenApi {
        api.title("LibLaaS-Web API")
            .summary("Provides API access to the dashboard.")
            .description("")
            .tag(Tag {
                name: "LibLaaS-Web".into(),
                description: Some("LibLaaS management".into()),
                ..Default::default()
            })
            .security_scheme(
                "Apikey",
                aide::openapi::SecurityScheme::ApiKey {
                    location: aide::openapi::ApiKeyLocation::Header,
                    name: "X-Auth-Key".into(),
                    description: Some("Key from dashboard".to_string()),
                    extensions: Default::default(),
                },
            )
            .default_response_with::<Json<ApiError>, _>(|res| {
                res.example(ApiError::trivial(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Default error, something has gone wrong".to_string(),
                ))
            })
    }

    let api_addr = config::settings().web.bind_addr.to_string();

    tracing::info!("Binding to {}", api_addr);
    let res = axum::Server::bind(
        &std::net::SocketAddr::from_str(&api_addr).expect("Expected api address as a string."),
    )
    .serve(app.into_make_service())
    .await;
    tracing::info!("Exited axum bind");
}
