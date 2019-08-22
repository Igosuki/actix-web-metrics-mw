# WARNING : Not ready for use (2019/08/22)

# actix-web-metrics-mw
Generic middleware library for actix-web metrics aggregation, can send to various outlets.

[![Build Status](https://travis-ci.org/nlopes/actix-web-metrics-mw.svg?branch=master)](https://travis-ci.org/Igosuki/actix-web-metrics-mw)
[![docs.rs](https://docs.rs/actix-web-metrics-mw/badge.svg)](https://docs.rs/actix-web-metrics-mw)
[![crates.io](https://img.shields.io/crates/v/actix-web-metrics-mw.svg)](https://crates.io/crates/actix-web-metrics-mw)
[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/nlopes/actix-web-metrics-mw/blob/master/LICENSE)

Metrics middleware instrumentation for [actix-web](https://github.com/actix/actix-web) using the [metrics-rs](https://crates.io/crates/metrics) crate .

By default two metrics are tracked (this assumes the namespace `actix_web_metrics_mw`):

Available exporters :
  - Statsd : supports the generic mode, or the dogstats mode, in dogstats mode the labels will
    be sent as tags in the metric name

Default metrics :
  - `http_requests_total` (labels: endpoint, method, status): request counter for each
   endpoint and method.
  - `http_requests_duration` (labels: endpoint, method,
   status): histogram of request durations for each endpoint.

## Usage

First add `actix_web_metrics_mw` to your `Cargo.toml`:

```toml
[dependencies]
actix_web_metrics_mw = "0.1"
```

You then instantiate the prometheus middleware and pass it to `.wrap()`:

```rust
use actix_web::{web, App, HttpResponse, HttpServer};
use actix_web_metrics_mw::Metrics;

fn health() -> HttpResponse {
    HttpResponse::Ok().finish()
}

fn main() -> std::io::Result<()> {
    let metrics = Metrics::new("/metrics", "actix_web_mw_test");
    HttpServer::new(move || {
        App::new()
            .wrap(metrics.clone())
            .service(web::resource("/health").to(health))
    })
    .bind("127.0.0.1:8080")?
    .run();
    Ok(())
}
```

Using the above as an example, a few things are worth mentioning:
 - `api` is the metrics namespace
 - `/metrics` will be auto exposed (GET requests only)

A call to the /metrics endpoint will expose your metrics:

```shell
$ curl http://localhost:8080/metrics
{"http_requests_total":"1570","http_requests_duration":"[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]"}
```

## Custom metrics

You can instantiate `Metrics` and then use its sink to register your custom
metric .

Then you can pass this counter through `.data()` to have it available within the resource
responder.

```rust
use actix_web::{web, App, HttpResponse, HttpServer};
use actix_web_prom::PrometheusMetrics;
use prometheus::IntCounterVec;

fn health(counter: web::Data<IntCounterVec>) -> HttpResponse {
    counter.with_label_values(&["endpoint", "method", "status"]).inc();
    HttpResponse::Ok().finish()
}

fn main() -> std::io::Result<()> {
    let prometheus = PrometheusMetrics::new("api", "/metrics");

    let counter_opts = opts!("counter", "some random counter").namespace("api");
    let counter = IntCounterVec::new(counter_opts, &["endpoint", "method", "status"]).unwrap();
    prometheus
        .registry
        .register(Box::new(counter.clone()))
        .unwrap();

    HttpServer::new(move || {
        App::new()
            .wrap(prometheus.clone())
            .data(counter.clone())
            .service(web::resource("/health").to(health))
    })
    .bind("127.0.0.1:8080")?
    .run();
    Ok(())
}
```

### Special Thanks

- The middleware integration is influenced by the work in [nlopes/actix-web-prom](https://github.com/nlopes/actix-web-prom).
