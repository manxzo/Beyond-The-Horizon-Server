use crate::handlers::auth::validate_jwt;
use actix_web::{
    Error, HttpMessage,
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    http::header,
};
use futures_util::future::{Ready, ok};
use std::{
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

/// Middleware for JWT authentication
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
        // Extract Authorization header and clone it to extend its lifetime
        let auth_header = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .map(String::from);

        let service = self.service.clone();

        // Move cloned auth_header into the async block
        Box::pin(async move {
            let token = match auth_header {
                Some(auth) => {
                    // Clone the token to prevent lifetime issues
                    let token_str = auth.clone();
                    match token_str.strip_prefix("Bearer ") {
                        Some(token) => token.to_string(), // Convert &str to owned String
                        None => {
                            return Err(actix_web::error::ErrorUnauthorized(
                                "Malformed auth header",
                            ));
                        }
                    }
                }
                None => return Err(actix_web::error::ErrorUnauthorized("Missing auth header")),
            };

            // Validate the JWT token
            match validate_jwt(&token) {
                Ok(claims) => {
                    // Store claims in request extensions
                    req.extensions_mut().insert(claims);
                    service.call(req).await
                }
                Err(_) => Err(actix_web::error::ErrorUnauthorized("Invalid token")),
            }
        })
    }
}
