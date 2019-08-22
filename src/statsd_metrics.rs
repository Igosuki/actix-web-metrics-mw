use metrics_core::{Builder, Drain, Key, Observe, Observer};
use std::borrow::BorrowMut;
use std::cell::RefCell;
use std::net::UdpSocket;
use std::ops::Deref;
use std::sync::atomic::AtomicPtr;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use cadence::ext::MetricBackend;
use cadence::prelude::*;
use cadence::{
    BufferedUdpMetricSink, MetricBuilder, QueuingMetricSink, StatsdClient, UdpMetricSink,
    DEFAULT_PORT,
};

/// Builder for [`StatsdObserver`].
#[derive(Clone)]
pub struct StatsdObserverBuilder {
    pub(crate) namespace: &'static str,
    pub(crate) endpoint: &'static str,
    pub(crate) port: u16,
}

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_NAMESPACE: &str = "statsd.test";

impl StatsdObserverBuilder {
    pub fn new() -> Self {
        Self {
            namespace: DEFAULT_NAMESPACE,
            endpoint: DEFAULT_HOST,
            port: DEFAULT_PORT,
        }
    }

    pub fn with_ns(mut self, namespace: &'static str) {
        self.namespace = namespace;
    }

    pub fn with_endpoint(mut self, endpoint: &'static str) {
        self.endpoint = endpoint;
    }

    pub fn with_port(mut self, port: u16) {
        self.port = port;
    }
}

impl Builder for StatsdObserverBuilder {
    type Output = StatsdObserver;

    fn build(&self) -> Self::Output {
        let socket = UdpSocket::bind("0.0.0.0:0").unwrap();
        socket.set_nonblocking(true).unwrap();

        let host = (self.endpoint, self.port);
        let udp_sink = BufferedUdpMetricSink::from(host, socket).unwrap();
        let queuing_sink = QueuingMetricSink::from(udp_sink);
        StatsdObserver {
            client: StatsdClient::from_sink(self.namespace, queuing_sink),
        }
    }
}

impl Default for StatsdObserverBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct StatsdObserver {
    client: StatsdClient,
}

//fn add_key_tags<T>(mut mb: Box<MetricBuilder<T>>, key: Key)
//where
//    T: cadence::Metric,
//    T: From<String>,
//{
//    for k in key.labels() {
//        mb.with_tag(k.key(), k.value());
//    }
//    mb.try_send();
//}

impl Observer for StatsdObserver {
    fn observe_counter(&mut self, key: Key, value: u64) {
        let name = key.name();
        let mut mb = self.client.count_with_tags(name.as_ref(), value as i64);
        for k in key.labels() {
            mb = mb.with_tag(k.key(), k.value());
        }
        mb.try_send();
        //        add_key_tags(mb, key);
    }

    fn observe_gauge(&mut self, key: Key, value: i64) {
        let name = key.name();
        let mut mb = self.client.gauge_with_tags(name.as_ref(), value as u64);
        for k in key.labels() {
            mb = mb.with_tag(k.key(), k.value());
        }
        mb.try_send();
        //        add_key_tags(mb, key);
    }

    fn observe_histogram(&mut self, key: Key, values: &[u64]) {
        let name = key.name();
        for value in values {
            let mut mb = self.client.histogram_with_tags(name.as_ref(), *value);
            for k in key.labels() {
                mb = mb.with_tag(k.key(), k.value());
            }
            mb.try_send();
            //            add_key_tags(mb, key);
        }
    }
}

impl Drain<String> for StatsdObserver {
    fn drain(&mut self) -> String {
        String::new()
    }
}

#[derive(Clone)]
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
    /// Creates a new [`StatsdExporter`] that logs events periodically
    ///
    /// Observers expose their output by being converted into strings.
    pub fn new(controller: C, builder: B, interval: Duration) -> Self {
        StatsdExporter {
            controller,
            observer: builder.build(),
            interval,
        }
    }

    /// Runs this exporter on the current thread, logging output at the interval
    /// given on construction.
    pub fn run(&mut self) {
        loop {
            thread::sleep(self.interval);

            self.turn();
        }
    }

    /// Run this exporter, logging output only once.
    pub fn turn(&mut self) {
        self.controller.observe(&mut self.observer);
        let output = self.observer.drain();
        log!(
            log::Level::Debug,
            "Metrics statsd exporter heartbeat {}",
            output
        );
    }

    pub fn get_controller(self) -> C {
        self.controller
    }
}
