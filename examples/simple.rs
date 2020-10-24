use actix_web::client::Client;
use actix_web::{web, App, Error, HttpServer};
use actix_web_metrics_mw::Metrics;

use actix_web::dev::Server;
use actix_web::web::BytesMut;
use core::result;
use futures::join;
use futures::StreamExt;
use futures_util::future::FutureExt;
use std::collections::HashMap;
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
        let handle: Server = rx.recv().unwrap();
        handle.stop(true).map(Ok)
    });

    match join!(
        async {
            let server = HttpServer::new(move || {
                App::new()
                    .wrap(metrics.clone())
                    .service(web::resource("/").to(|| async {
                        "Hello, middleware! Check the console where the server is run."
                    }))
            })
            .bind("127.0.0.1:8080")?
            .run();
            let _ = tx.send(server.clone());
            server.await
        },
        producer
    ) {
        (_, Err(e)) => Err(e),
        (Err(e), _) => Err(Error::from(e)),
        (_, _) => Ok(()),
    }
}

fn parse_assert_metrics(body: &BytesMut) {
    let json_result: HashMap<String, &str> = serde_json::from_slice(body).unwrap();
    let requests_total = json_result.get("http_requests_total").unwrap();
    println!("Total http requests : {:?}", requests_total);
    assert_eq!(*requests_total, "10");
    let histo = *json_result.get("http_requests_duration").unwrap();
    let histo_vec: Vec<u64> = serde_json::from_str(histo).unwrap();
    println!("Request time histogram : {:?}", histo_vec);
    assert!(*histo_vec.first().unwrap() > 10);
}

async fn producer_loop() -> Result<(), Error> {
    let client = Client::new();
    let mut loops: i32 = 0;
    while loops < 10 {
        let _ = client.get("http://localhost:8080/").send().await;
        loops += 1;
    }
    let mut resp = client
        .get("http://localhost:8080/metrics")
        .send()
        .await
        .map_err(|e| {
            println!("{:?}", e);
            Error::from(e)
        })?;
    let mut body = BytesMut::new();
    while let Some(chunk) = resp.next().await {
        body.extend_from_slice(&chunk?);
    }
    Ok(parse_assert_metrics(&body))
}
