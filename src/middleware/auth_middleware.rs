use crate::handlers::auth::Claims;
use actix_identity::Identity;
use actix_web::{
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpMessage,
};
use futures_util::future::{ok, Ready};
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
            // Get identity from the request
            let authenticated = if let Some(id) = req.extensions().get::<Identity>() {
                // Get user claims from identity
                match id.id() {
                    Ok(claims_str) => {
                        // Deserialize the claims
                        match from_str::<Claims>(&claims_str) {
                            Ok(claims) => {
                                // Store claims in request extensions
                                req.extensions_mut().insert(claims);
                                true
                            }
                            Err(_) => {
                                return Err(actix_web::error::ErrorUnauthorized(
                                    "Invalid session data",
                                ));
                            }
                        }
                    }
                    Err(_) => {
                        return Err(actix_web::error::ErrorUnauthorized(
                            "Session expired or invalid",
                        ));
                    }
                }
            } else {
                return Err(actix_web::error::ErrorUnauthorized("Not authenticated"));
            };

            if authenticated {
                service.call(req).await
            } else {
                Err(actix_web::error::ErrorUnauthorized("Authentication failed"))
            }
        })
    }
}
