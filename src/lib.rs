#[macro_use]
extern crate log;
extern crate metrics;
extern crate metrics_core;
extern crate metrics_runtime;
extern crate statsd;

use std::thread;
use actix_service::{Service, Transform};
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::{http, Error, HttpResponse};
use futures::future::{ok, Either, FutureResult};
use futures::Poll;
use metrics_runtime::{Receiver, Controller};
use std::time::Duration;
use statsd_metrics::{StatsdExporter, StatsdObserverBuilder};
use std::{sync::Arc};
use std::borrow::BorrowMut;

mod statsd_metrics;

pub struct Metrics {
    pub(crate) namespace: String,
    pub(crate) endpoint: String,
    exporter: StatsdExporter<Controller, StatsdObserverBuilder>,
    receiver: Arc<Receiver>,
}

impl Metrics {
    /// Create a new Metrics. You set the namespace and the metrics endpoint
    /// through here.
    pub fn new(namespace: &str, endpoint: &str) -> Self
    {
        let receiver = Arc::from(Receiver::builder().build().expect("failed to create receiver"));
        let exporter = StatsdExporter::new(receiver.get_controller().clone(), StatsdObserverBuilder::new(), Duration::from_secs(5));
        Metrics {
            namespace: namespace.to_string(),
            endpoint: endpoint.to_string(),
            exporter,
            receiver,
        }
    }
    pub fn start(mut self) {
        self.receiver.install();
        thread::spawn(|| self.exporter.borrow_mut().run());
    }
}

impl<S, B> Transform<S> for Metrics
    where
        S: Service<Request=ServiceRequest, Response=ServiceResponse<B>, Error=Error>,
        S::Future: 'static,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = MetricsMiddleware<S>;
    type Future = FutureResult<Self::Transform, Self::InitError>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(MetricsMiddleware { service })
    }
}

pub struct MetricsMiddleware<S> {
    service: S,
}

impl<S, B> Service for MetricsMiddleware<S>
    where
        S: Service<Request=ServiceRequest, Response=ServiceResponse<B>, Error=Error>,
        S::Future: 'static,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = Either<S::Future, FutureResult<Self::Response, Self::Error>>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready()
    }

    fn call(&mut self, req: ServiceRequest) -> Self::Future {
// We only need to hook into the `start` for this middleware.

        let is_logged_in = false; // Change this to see the change in outcome in the browser

        if is_logged_in {
            Either::A(self.service.call(req))
        } else {
// Don't forward to /login if we are already on /login
            if req.path() == "/login" {
                Either::A(self.service.call(req))
            } else {
                Either::B(ok(req.into_response(
                    HttpResponse::Found()
                        .header(http::header::LOCATION, "/login")
                        .finish()
                        .into_body(),
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::test::{call_service, init_service, read_body, read_response, TestRequest};
    use actix_web::{web, App, HttpResponse};

    #[test]
    fn middleware_basic() {
        let metrics = Metrics::new("actix_web_mw_test", "/metrics");

        let mut app = init_service(
            App::new()
                .wrap(metrics)
                .service(web::resource("/health_check").to(|| HttpResponse::Ok())),
        );

        let res = call_service(
            &mut app,
            TestRequest::with_uri("/health_check").to_request(),
        );
        assert!(res.status().is_success());
        assert_eq!(read_body(res), "");

        let res = read_response(&mut app, TestRequest::with_uri("/metrics").to_request());
        let body = String::from_utf8(res.to_vec()).unwrap();
        assert!(&body.contains(
            &String::from_utf8(web::Bytes::from(
                r#"{"server": {"requests": {"omegalul": { "count": 1 } } } }"#
            ).to_vec()).unwrap()));
    }
}
