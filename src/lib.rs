#[macro_use]
extern crate log;
extern crate metrics;
extern crate metrics_core;
extern crate metrics_runtime;
extern crate statsd;

use actix_service::{Service, Transform};
use actix_web::{
    dev::{Body, BodySize, MessageBody, ResponseBody, ServiceRequest, ServiceResponse},
    http::{Method, StatusCode},
    web::Bytes,
    web::Json,
    Error,
};
use actix_web::{http, HttpResponse};
use futures::future::{ok, Either, FutureResult};
use futures::{Async, Future, Poll};
use metrics::Recorder;
use metrics_core::{Key, Label};
use metrics_runtime::data::Snapshot;
use metrics_runtime::{AsScoped, Controller, Receiver};
use serde_json;
use statsd_metrics::{StatsdExporter, StatsdObserverBuilder};
use std::borrow::Cow;
use std::borrow::{Borrow, BorrowMut};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime};

mod statsd_metrics;

pub struct Metrics {
    pub(crate) namespace: String,
    pub(crate) path: String,
    exporter: StatsdExporter<Controller, StatsdObserverBuilder>,
    receiver: Box<Receiver>,
}

impl Metrics {
    /// Create a new Metrics. You set the namespace and the metrics endpoint
    /// through here.
    pub fn new(path: &str, namespace: &str) -> Self {
        let receiver = Box::new(
            Receiver::builder()
                .build()
                .expect("failed to create receiver"),
        );
        let controller = receiver.get_controller();
        let exporter = StatsdExporter::new(
            controller.clone(),
            StatsdObserverBuilder::new(),
            Duration::from_secs(5),
        );
        Metrics {
            namespace: namespace.to_string(),
            path: path.to_string(),
            exporter,
            receiver,
        }
    }

    fn update_metrics(&self, path: &str, method: &Method, status: StatusCode, clock: SystemTime) {
        let method = method.to_string();
        let status = status.as_u16().to_string();
        let labels: Vec<Label> = vec![
            ("path", "").into(),
            Label::new("method", ""),
            Label::new("status", ""),
        ];
        if let Ok(elapsed) = clock.elapsed() {
            let duration =
                (elapsed.as_secs() as f64) + f64::from(elapsed.subsec_nanos()) / 1_000_000_000_f64;
            self.receiver.record_histogram(
                Key::from_name_and_labels("http_requests_duration", labels),
                duration as u64,
            );
        }
        self.receiver.record_counter(
            Key::from_name_and_labels(
                "http_requests_total",
                vec![
                    Label::new("path", ""),
                    Label::new("method", ""),
                    Label::new("status", ""),
                ],
            ),
            1,
        );
    }

    fn metrics(&self) -> String {
        let snapshot: Snapshot = self.receiver.get_controller().snapshot();
        let metrics: HashMap<String, String> = snapshot
            .into_measurements()
            .iter()
            .map(|(k, v)| (k.to_string(), "".to_string()))
            .collect();
        serde_json::to_string(&metrics).unwrap()
    }

    fn matches(&self, path: &str, method: &Method) -> bool {
        self.path == path && method == Method::GET
    }

    pub fn start(mut self) {
        thread::spawn(move || {
            self.receiver.install();
            self.exporter.borrow_mut().run()
        });
    }
}

impl<S, B> Transform<S> for Metrics
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: MessageBody,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<StreamLog<B>>;
    type Error = Error;
    type InitError = ();
    type Transform = MetricsMiddleware<S>;
    type Future = FutureResult<Self::Transform, Self::InitError>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(MetricsMiddleware {
            service,
            inner: Arc::new(self.clone()),
        })
    }
}

#[doc(hidden)]
pub struct MetricsResponse<S, B>
where
    B: MessageBody,
    S: Service,
{
    fut: S::Future,
    clock: SystemTime,
    inner: Arc<Metrics>,
    _t: PhantomData<(B,)>,
}

impl<S, B> Future for MetricsResponse<S, B>
where
    B: MessageBody,
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
{
    type Item = ServiceResponse<StreamLog<B>>;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let res = futures::try_ready!(self.fut.poll());

        let req = res.request();
        let inner = self.inner.clone();
        let method = req.method().clone();
        let path = req.path().to_string();

        Ok(Async::Ready(res.map_body(move |mut head, mut body| {
            // We short circuit the response status and body to serve the endpoint
            // automagically. This way the user does not need to set the middleware *AND*
            // an endpoint to serve middleware results. The user is only required to set
            // the middleware and tell us what the endpoint should be.
            if inner.matches(&path, &method) {
                head.status = StatusCode::OK;
                body = ResponseBody::Other(Body::from_message(inner.metrics()));
            }
            ResponseBody::Body(StreamLog {
                body,
                size: 0,
                clock: self.clock,
                inner,
                status: head.status,
                path,
                method,
            })
        })))
    }
}

#[doc(hidden)]
pub struct StreamLog<B> {
    body: ResponseBody<B>,
    size: usize,
    clock: SystemTime,
    inner: Arc<Metrics>,
    status: StatusCode,
    path: String,
    method: Method,
}

impl<B> Drop for StreamLog<B> {
    fn drop(&mut self) {
        // update the metrics for this request at the very end of responding
        self.inner
            .update_metrics(&self.path, &self.method, self.status, self.clock);
    }
}

impl<B: MessageBody> MessageBody for StreamLog<B> {
    fn size(&self) -> BodySize {
        self.body.size()
    }

    fn poll_next(&mut self) -> Poll<Option<Bytes>, Error> {
        match self.body.poll_next()? {
            Async::Ready(Some(chunk)) => {
                self.size += chunk.len();
                Ok(Async::Ready(Some(chunk)))
            }
            val => Ok(val),
        }
    }
}

pub struct MetricsMiddleware<S> {
    service: S,
    inner: Arc<Metrics>,
}

impl<S, B> Service for MetricsMiddleware<S>
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: MessageBody,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<StreamLog<B>>;
    type Error = Error;
    type Future = MetricsResponse<S, B>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready()
    }

    fn call(&mut self, req: ServiceRequest) -> Self::Future {
        MetricsResponse {
            fut: self.service.call(req),
            clock: SystemTime::now(),
            inner: self.inner.clone(),
            _t: PhantomData,
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
        let metrics = Metrics::new("/metrics", "actix_web_mw_test");

        let mut app = init_service(
            App::new()
                .wrap(metrics)
                .service(web::resource("/health_check").to(|| HttpResponse::Ok())),
        );

        let res = call_service(
            &mut app,
            TestRequest::with_uri("/health_check").to_request(),
        );
        println!("{}", res.status());
        assert!(res.status().is_success());
        assert_eq!(read_body(res), "");

        let res = read_response(&mut app, TestRequest::with_uri("/metrics").to_request());
        let body = String::from_utf8(res.to_vec()).unwrap();
        assert!(&body.contains(
            &String::from_utf8(
                web::Bytes::from(r#"{"server": {"requests": {"omegalul": { "count": 1 } } } }"#)
                    .to_vec()
            )
            .unwrap()
        ));
    }
}
