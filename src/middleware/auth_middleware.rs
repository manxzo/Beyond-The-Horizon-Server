use crate::handlers::auth::Claims;
use actix_identity::Identity;
use actix_web::{
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpMessage,web,
};
use futures_util::future::{ok, Ready};
use log::{error, info};
use serde_json::from_str;
use std::{
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

/// Middleware for session-based authentication
pub struct AuthMiddleware;

impl<S, B> Transform<S, ServiceRequest> for AuthMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = AuthMiddlewareMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(AuthMiddlewareMiddleware {
            service: Rc::new(service),
        })
    }
}

pub struct AuthMiddlewareMiddleware<S> {
    pub service: Rc<S>,
}

impl<S, B> Service<ServiceRequest> for AuthMiddlewareMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(&self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(ctx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = self.service.clone();

        Box::pin(async move {
            // First try to authenticate with the session cookie
            let cookie_auth = if let Some(id) = req.extensions().get::<Identity>() {
                match id.id() {
                    Ok(claims_str) => {
                        info!("Found identity with claims: {}", claims_str);

                        match from_str::<Claims>(&claims_str) {
                            Ok(claims) => {
                                info!(
                                    "Successfully authenticated user via cookie: {}",
                                    claims.username
                                );
                                req.extensions_mut().insert(claims);
                                true
                            }
                            Err(e) => {
                                error!("Failed to deserialize claims: {}", e);
                                false
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to get identity ID: {}", e);
                        false
                    }
                }
            } else {
                false
            };

            // If cookie auth failed, try JWT token auth
            if !cookie_auth {
                // Check for Authorization header
                if let Some(auth_header) = req.headers().get("Authorization") {
                    if let Ok(auth_str) = auth_header.to_str() {
                        if auth_str.starts_with("Bearer ") {
                            let token = auth_str.trim_start_matches("Bearer ").trim();

                            // Get the session secret
                            let session_secret = req
                                .app_data::<web::Data<String>>()
                                .map(|data| data.get_ref().clone())
                                .unwrap_or_else(|| "default_session_secret".to_string());

                            // Verify and decode the token
                            match jsonwebtoken::decode::<Claims>(
                                token,
                                &jsonwebtoken::DecodingKey::from_secret(session_secret.as_bytes()),
                                &jsonwebtoken::Validation::default(),
                            ) {
                                Ok(token_data) => {
                                    info!(
                                        "Successfully authenticated user via JWT: {}",
                                        token_data.claims.username
                                    );
                                    req.extensions_mut().insert(token_data.claims);
                                    return service.call(req).await;
                                }
                                Err(e) => {
                                    error!("JWT validation failed: {}", e);
                                }
                            }
                        }
                    }
                }

                // If we get here, both auth methods failed
                if !cookie_auth {
                    error!("No valid authentication found");
                    return Err(actix_web::error::ErrorUnauthorized("Authentication failed"));
                }
            }

            // If we get here, cookie auth succeeded
            service.call(req).await
        })
    }
}
