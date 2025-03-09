use crate::handlers::auth::Claims;
use actix_identity::Identity;
use actix_web::{
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpMessage,
};
use chrono::Utc;
use futures_util::future::{ok, Ready};
use serde_json::to_string;
use std::{
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

/// Middleware for automatically refreshing sessions that are close to expiring
pub struct SessionRefreshMiddleware {
    /// Threshold in seconds before expiration to refresh the session
    refresh_threshold: u64,
}

impl SessionRefreshMiddleware {
    pub fn new(refresh_threshold_seconds: u64) -> Self {
        SessionRefreshMiddleware {
            refresh_threshold: refresh_threshold_seconds,
        }
    }
}

impl<S, B> Transform<S, ServiceRequest> for SessionRefreshMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = SessionRefreshMiddlewareService<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(SessionRefreshMiddlewareService {
            service: Rc::new(service),
            refresh_threshold: self.refresh_threshold,
        })
    }
}

pub struct SessionRefreshMiddlewareService<S> {
    service: Rc<S>,
    refresh_threshold: u64,
}

impl<S, B> Service<ServiceRequest> for SessionRefreshMiddlewareService<S>
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
        let refresh_threshold = self.refresh_threshold;

        Box::pin(async move {
            // Check if there's an identity in the request
            if let Some(identity) = req.extensions().get::<Identity>() {
                if let Ok(claims_str) = identity.id() {
                    // Try to deserialize the claims
                    if let Ok(mut claims) = serde_json::from_str::<Claims>(&claims_str) {
                        // Get current time
                        let now = Utc::now().timestamp() as usize;

                        // Check if session is close to expiring
                        if claims.exp > now && claims.exp - now < refresh_threshold as usize {
                            // Create new expiration time
                            let new_exp = Utc::now().timestamp() as usize + (12 * 60 * 60); // 12 hours
                            claims.exp = new_exp;

                            // Serialize updated claims
                            if let Ok(updated_claims_str) = to_string(&claims) {
                                // Update the identity with new expiration
                                let _ = Identity::login(&req.extensions(), updated_claims_str);
                            }
                        }
                    }
                }
            }

            // Continue with the request
            service.call(req).await
        })
    }
}
