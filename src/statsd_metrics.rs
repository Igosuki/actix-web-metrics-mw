use metrics_core::{Builder, Drain, Key, Observe, Observer};
use std::borrow::BorrowMut;
use std::cell::RefCell;
use std::ops::Deref;
use std::sync::atomic::AtomicPtr;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use cadence::prelude::*;
use cadence::{StatsdClient, UdpMetricSink, DEFAULT_PORT};

/// Builder for [`StatsdObserver`].
#[derive(Clone)]
pub struct StatsdObserverBuilder {
    pub(crate) namespace: &'static str,
    pub(crate) endpoint: &'static str,
}

const DEFAULT_STATSD_URL: &str = "127.0.0.1:8125";
const DEFAULT_NAMESPACE: &str = "myapp";

impl StatsdObserverBuilder {
    pub fn new() -> Self {
        Self {
            namespace: DEFAULT_NAMESPACE,
            endpoint: DEFAULT_STATSD_URL,
        }
    }

    pub fn set_client(mut self, endpoint: &'static str, namespace: &'static str) {
        self.namespace = namespace;
        self.endpoint = endpoint;
    }
}

impl Builder for StatsdObserverBuilder {
    type Output = StatsdObserver;

    fn build(&self) -> Self::Output {
        StatsdObserver {
            client: StatsdClient::from_udp_host(self.namespace, self.endpoint).unwrap(),
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

impl Observer for StatsdObserver {
    fn observe_counter(&mut self, key: Key, value: u64) {
        self.client.count(key.to_string().as_str(), value as i64);
    }

    fn observe_gauge(&mut self, _key: Key, _value: i64) {
        unimplemented!()
    }

    fn observe_histogram(&mut self, _key: Key, _values: &[u64]) {
        unimplemented!()
    }
}

impl Drain<String> for StatsdObserver {
    fn drain(&mut self) -> String {
        unimplemented!()
    }
}

#[derive(Clone, Copy)]
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
        log!(log::Level::Info, "{}", output);
    }

    pub fn get_controller(self) -> C {
        self.controller
    }
}
