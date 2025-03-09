use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error,
};
use chrono::Utc;
use futures::future::{ok, LocalBoxFuture, Ready};
use log::{error, info};
use std::{rc::Rc, time::Instant};

// Request logger middleware
pub struct RequestLogger;

impl<S, B> Transform<S, ServiceRequest> for RequestLogger
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = RequestLoggerMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(RequestLoggerMiddleware {
            service: Rc::new(service),
        })
    }
}

pub struct RequestLoggerMiddleware<S> {
    service: Rc<S>,
}

impl<S, B> Service<ServiceRequest> for RequestLoggerMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let start_time = Instant::now();
        let method = req.method().clone();
        let path = req.path().to_owned();
        let connection_info = req.connection_info().clone();
        let client_ip = connection_info.peer_addr().unwrap_or("unknown").to_owned();
        let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string();

        // Log request details
        info!(
            "[BTH-REQUEST] {} - {} {} - Client IP: {} - Timestamp: {}",
            client_ip, method, path, client_ip, timestamp
        );

        let service = self.service.clone();
        Box::pin(async move {
            let res = service.call(req).await;
            let elapsed = start_time.elapsed();

            match &res {
                Ok(response) => {
                    let status = response.status();
                    info!(
                        "[BTH-RESPONSE] {} - {} {} - Status: {} - Time: {:.2?} - Timestamp: {}",
                        client_ip,
                        method,
                        path,
                        status.as_u16(),
                        elapsed,
                        timestamp
                    );
                }
                Err(err) => {
                    error!(
                        "[BTH-ERROR] {} - {} {} - Error: {} - Time: {:.2?} - Timestamp: {}",
                        client_ip, method, path, err, elapsed, timestamp
                    );
                }
            }

            res
        })
    }
}
