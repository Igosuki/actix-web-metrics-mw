#[macro_use]
extern crate log;
extern crate cadence;
extern crate metrics;
#[macro_use]
extern crate metrics_core;
extern crate metrics_runtime;
extern crate pin_project;

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime};

use actix_service::{Service, Transform};
use actix_web::{
    dev::{Body, BodySize, MessageBody, ResponseBody, ServiceRequest, ServiceResponse},
    http::{Method, StatusCode},
    web::Bytes,
    Error,
};
use futures::future::{ok, Ready};
use futures::task::{Context, Poll};
use futures::Future;
use metrics_core::Label;
use metrics_runtime::Measurement;
use metrics_runtime::{Controller, Receiver, Sink};
use pin_project::{pin_project, pinned_drop};
use serde_json;

use statsd_metrics::{StatsdExporter, StatsdObserverBuilder};

mod statsd_metrics;

#[derive(Clone)]
#[must_use = "must be set up as a middleware for actix-web"]
/// By default two metrics are tracked (this assumes the namespace `actix_web_prom`):
///
/// This uses the generic metrics crate which allows you to :
///   - Push histograms, gauges and counters to a receiver
///   - Register an exporter that will periodically drain the latest metrics
///
/// Available exporters :
///   - Statsd : supports the generic mode, or the dogstats mode, in dogstats mode the labels will
///     be sent as tags in the metric name
///
/// Default metrics :
///   - `http_requests_total` (labels: endpoint, method, status): request counter for each
///    endpoint and method.
///
///   - `http_requests_duration` (labels: endpoint, method,
///    status): histogram of request durations for each endpoint.
pub struct Metrics {
    pub(crate) namespace: String,
    pub(crate) path: String,
    exporter: Box<StatsdExporter<Controller, StatsdObserverBuilder>>,
    sink: Sink,
    default_labels: Vec<String>,
}

fn to_scoped((k, v): (&str, &str)) -> Label {
    Label::new(Cow::from(k).into_owned(), Cow::from(v).into_owned())
}

impl Metrics {
    /// Create a new Metrics. You set the namespace and the metrics endpoint
    /// through here.
    pub fn new(path: &str, namespace: &str, labels: Vec<(&str, &str)>) -> Self {
        let receiver = Receiver::builder()
            .build()
            .expect("failed to create receiver");
        let controller = receiver.controller();
        let exporter = StatsdExporter::new(
            controller.clone(),
            StatsdObserverBuilder::new(),
            Duration::from_secs(5),
        );
        let mut sink = receiver.sink();
        let x: Vec<Label> = labels.iter().map(|&kv| to_scoped(kv)).collect();
        sink.add_default_labels(x);
        let m = Metrics {
            namespace: namespace.to_string(),
            path: path.to_string(),
            exporter: Box::new(exporter),
            sink,
            default_labels: vec![],
        };
        receiver.install();
        m
    }

    fn update_metrics(&self, path: &str, method: &Method, status: StatusCode, clock: SystemTime) {
        let p = Cow::from(path).into_owned();
        let m = Cow::from(method.as_str()).into_owned();
        let st = Cow::from(status.as_str()).into_owned();
        let labels: Vec<Label> = labels!("path" => p, "method" => m, "status" => st);
        if let Ok(elapsed) = clock.elapsed() {
            let duration = (elapsed.as_micros() as u64) + elapsed.subsec_micros() as u64;
            self.sink
                .clone()
                .histogram_with_labels("http_requests_duration", labels.clone())
                .record_value(duration);
        }
        self.sink
            .clone()
            .counter_with_labels("http_requests_total", labels.clone())
            .record(1);
    }

    fn metrics(&self) -> String {
        let x = self.exporter.clone().get_controller();
        let snapshot = x.snapshot();
        let metrics: BTreeMap<String, String> = snapshot
            .into_measurements()
            .iter()
            .map(|(k, v)| (format!("{}", k.name()), Metrics::print_measure(v)))
            .collect();
        serde_json::to_string(&metrics).unwrap()
    }

    fn print_measure(v: &Measurement) -> String {
        match v {
            Measurement::Counter(a) => a.to_string(),
            Measurement::Gauge(g) => g.to_string(),
            Measurement::Histogram(h) => format!("{:?}", h.decompress()),
        }
    }

    fn matches(&self, path: &str, method: &Method) -> bool {
        self.path == path && method == Method::GET
    }

    pub fn start(mut self) {
        thread::spawn(move || self.exporter.run());
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
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(MetricsMiddleware {
            service,
            inner: Arc::new(self.clone()),
        })
    }
}

#[doc(hidden)]
#[pin_project::pin_project]
pub struct MetricsResponse<S, B>
where
    B: MessageBody,
    S: Service,
{
    #[pin]
    fut: S::Future,
    clock: SystemTime,
    #[pin]
    inner: Arc<Metrics>,
    _t: PhantomData<(B,)>,
}

impl<S, B> Future for MetricsResponse<S, B>
where
    B: MessageBody,
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
{
    type Output = Result<ServiceResponse<StreamLog<B>>, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let res = match futures::ready!(this.fut.poll(cx)) {
            Ok(res) => res,
            Err(e) => return Poll::Ready(Err(e)),
        };

        let req = res.request();
        let method = req.method().clone();
        let path = req.path().to_string();
        let inner = this.inner.clone();
        let metrics = this.inner.metrics();
        let clock = this.clock.clone();
        Poll::Ready(Ok(res.map_body(move |mut head, mut body| {
            // We short circuit the response status and body to serve the endpoint
            // automagically. This way the user does not need to set the middleware *AND*
            // an endpoint to serve middleware results. The user is only required to set
            // the middleware and tell us what the endpoint should be.

            if inner.matches(&path, &method) {
                head.status = StatusCode::OK;
                body = ResponseBody::Other(Body::from_message(metrics));
            }
            ResponseBody::Body(StreamLog {
                body,
                size: 0,
                clock,
                inner,
                status: head.status,
                path,
                method,
            })
        })))
    }
}

#[doc(hidden)]
#[pin_project(PinnedDrop)]
pub struct StreamLog<B> {
    #[pin]
    body: ResponseBody<B>,
    size: usize,
    clock: SystemTime,
    inner: Arc<Metrics>,
    status: StatusCode,
    path: String,
    method: Method,
}

#[pinned_drop]
impl<B> PinnedDrop for StreamLog<B> {
    fn drop(self: Pin<&mut Self>) {
        // update the metrics for this request at the very end of responding
        self.inner
            .update_metrics(&self.path, &self.method, self.status, self.clock);
    }
}

impl<B: MessageBody> MessageBody for StreamLog<B> {
    fn size(&self) -> BodySize {
        self.body.size()
    }

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Result<Bytes, Error>>> {
        let this = self.project();
        match this.body.poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                *this.size += chunk.len();
                Poll::Ready(Some(Ok(chunk)))
            }
            val => val,
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

    fn poll_ready(&mut self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(ctx)
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
    use std::collections::HashMap;

    #[actix_rt::test]
    async fn middleware_basic() {
        let metrics = Metrics::new(
            "/metrics",
            "actix_web_mw_test",
            vec![("test_label", "test_value")],
        );

        let mut app = init_service(
            App::new()
                .wrap(metrics)
                .service(web::resource("/health_check").to(|| HttpResponse::Ok())),
        )
        .await;

        let res = call_service(
            &mut app,
            TestRequest::with_uri("/health_check").to_request(),
        )
        .await;
        assert!(res.status().is_success());
        let body1 = read_body(res).await;
        assert_eq!(body1, "");

        let res = read_response(&mut app, TestRequest::with_uri("/metrics").to_request()).await;
        let body = String::from_utf8(res.to_vec()).unwrap();
        println!("{}", body);
        let json_result: HashMap<String, &str> = serde_json::from_str(body.as_str()).unwrap();
        assert_eq!(*json_result.get("http_requests_total").unwrap(), "1");
        let histo = *json_result.get("http_requests_duration").unwrap();
        let histo_vec: Vec<u64> = serde_json::from_str(histo).unwrap();
        assert!(*histo_vec.first().unwrap() > 10);
    }
}
