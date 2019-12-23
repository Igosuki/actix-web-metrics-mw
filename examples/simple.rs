use actix_web::client::{Client};
use actix_web::{web, App, Error, HttpServer};
use actix_web_metrics_mw::Metrics;

use actix_web::dev::Server;
use core::result;
use futures::{join};
use std::collections::HashMap;
use futures_util::future::TryFutureExt;
use futures_util::future::FutureExt;
use std::sync::mpsc;

#[actix_rt::main]
async fn main() -> result::Result<(), Error> {
    std::env::set_var("RUST_LOG", "actix_web=debug");

    let metrics = Metrics::new(
        "/metrics",
        "actix_web_mw_test",
        vec![("test_label", "test_value")],
    );
    let (tx, rx) = mpsc::channel();

    let producer = producer_loop().then(move |_e| {
        let handle : Server = rx.recv().unwrap();
        handle.stop(true).map(Ok)
    });

    match join!(async {
        let server = HttpServer::new(move || {
            App::new().wrap(metrics.clone()).service(
                web::resource("/")
                    .to(|| async { "Hello, middleware! Check the console where the server is run." }),
            )
        })
        .bind("127.0.0.1:8080")
        .unwrap()
        .start();
        let _ = tx.send(server.clone());
        server.await
    }, producer) {
        (_, Err(e)) => Err(e),
        (Err(e), _) => Err(Error::from(e)),
        (_, _) => Ok(()),
    }
}

fn parse_assert_metrics(body: String) {
    let json_result: HashMap<String, &str> = serde_json::from_str(body.as_str()).unwrap();
    assert_eq!(*json_result.get("http_requests_total").unwrap(), "1");
    let histo = *json_result.get("http_requests_duration").unwrap();
    let histo_vec: Vec<u64> = serde_json::from_str(histo).unwrap();
    assert!(*histo_vec.first().unwrap() > 10);
}

async fn producer_loop() -> Result<(), Error> {
    let client = Client::new();
    let mut loops: i32 = 0;
    while loops < 10 {
        let _ = client.get("http://localhost:8080/").send().await;
        loops += 1;
    }
    let resp = client.get("/metrics").send().await.map_err(|e| {
        println!("{:?}", e);
        Error::from(e)
    })?;
    Ok(parse_assert_metrics(format!("{:?}", resp)))
}
