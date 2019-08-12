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
use metrics::{Recorder};
use metrics_core::{Key, Builder, Drain, Observer, Observe};
use metrics::SetRecorderError;
use std::ops::Deref;
use metrics_runtime::{Receiver, Sink, Controller};
use std::time::Duration;
use statsd::Client;

/// Builder for [`StatsdObserver`].
pub struct StatsdObserverBuilder {
    pub(crate) client: statsd::Client,
}

impl StatsdObserverBuilder {
    pub fn new() -> Self {
        Self{
            client: Client::new("127.0.0.1:8125", "myapp").unwrap(),
        }
    }

    fn client(mut self, client: statsd::Client) -> Self {
        self.client = client;
        self
    }
}

impl Builder for StatsdObserverBuilder {
    type Output = StatsdObserver;

    fn build(&self) -> Self::Output {
        StatsdObserver {
            client: self.client,
        }
    }
}

impl Default for StatsdObserverBuilder {
    fn default() -> Self {
        Self::new()
    }
}

pub struct StatsdObserver {
    client: statsd::Client,
}

impl Observer for StatsdObserver {
    fn observe_counter(&mut self, key: Key, value: u64) {
        unimplemented!()
    }

    fn observe_gauge(&mut self, key: Key, value: i64) {
        unimplemented!()
    }

    fn observe_histogram(&mut self, key: Key, values: &[u64]) {
        unimplemented!()
    }
}

impl Drain<String> for StatsdObserver {
    fn drain(&mut self) -> String {
        unimplemented!()
    }
}

pub struct StatsdExporter<C, B>
    where
        B: Builder,
{
    controller: C,
    observer: B::Output,
    interval: Duration,
}

impl<C, B> StatsdExporter<C, B>
    where
        B: Builder,
        B::Output: Drain<String> + Observer,
        C: Observe,
{
    /// Creates a new [`LogExporter`] that logs at the configurable level.
    ///
    /// Observers expose their output by being converted into strings.
    pub fn new(controller: C, builder: B, interval: Duration) -> Self {
        StatsdExporter {
            controller,
            observer: builder.statsd_client(Client::new("127.0.0.1:8125", "myapp").unwrap()).build(),
            interval,
        }
    }

    /// Runs this exporter on the current thread, logging output at the interval
    /// given on construction.
    pub fn run(self) {
        loop {
            thread::sleep(self.interval);

            self.turn();
        }
    }

    /// Run this exporter, logging output only once.
    pub fn turn(self) {
        self.controller.observe(&mut self.observer);
        let output = self.observer.drain();
    }

}

pub struct Metrics {
    pub(crate) namespace: String,
    pub(crate) endpoint: String,
    exporter: StatsdExporter<Controller, StatsdObserverBuilder>,
}

static receiver : Receiver = Receiver::builder().build().expect("failed to create receiver");

impl Metrics {
    /// Create a new Metrics. You set the namespace and the metrics endpoint
    /// through here.
    pub fn new(namespace: &str, endpoint: &str) -> Self
    {
        let exporter = StatsdExporter::new(receiver.get_controller().clone(), StatsdObserverBuilder::new(), Duration::from_secs(5));
        Metrics {
            namespace: namespace.to_string(),
            endpoint: endpoint.to_string(),
            exporter,
        }
    }
    pub fn start(&self) {
        thread::spawn(move || self.exporter.run());
        receiver.install();
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