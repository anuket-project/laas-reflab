//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

#![allow(dead_code, unused_variables)]

use crossbeam_channel::Sender;

use std::{
    marker::PhantomData,
    sync::{Condvar, Mutex},
    thread::JoinHandle,
};

pub mod prelude {
    pub use aide;
    pub use anyhow;
    pub use async_recursion;
    pub use axum;
    pub use axum_extra;
    pub use axum_jsonschema;
    pub use chrono;
    pub use config;
    pub use crossbeam_channel;
    pub use dashmap;
    pub use dotenv;
    pub use futures;
    pub use http_body;
    pub use hyper;
    pub use inquire;
    pub use itertools;
    pub use lazy_static;
    pub use macaddr;
    pub use once_cell;
    pub use parking_lot;
    pub use parse_size;
    pub use rand;
    pub use rayon;
    pub use regex;
    pub use reqwest;
    pub use schemars;
    pub use serde_json;
    pub use serde_with;
    pub use serde_yaml;
    pub use strum;
    pub use strum_macros;
    pub use thiserror;
    pub use tokio;
    pub use tokio_postgres;
    pub use tower;
    pub use tower_http;
    pub use tracing;

    pub use serde::{Deserialize, Serialize};
}

/// This is used to get around PhantomData<V> not being Send/Sync for arbitrary V
struct PhantomSendParams<Req, Resp, Handler> {
    _pd: PhantomData<(Req, Resp, Handler)>,
}

impl<A, B, C> PhantomSendParams<A, B, C> {
    /// Unsafe: this is only safe if this is purely a marker,
    /// and isn't being relied on to actually mark *real*
    /// instances of <A, B, C>, say, behind a pointer.
    ///
    /// This is just for "nice generics", not for using
    /// for black magic
    pub unsafe fn new() -> Self {
        Self {
            _pd: PhantomData::default(),
        }
    }
}

unsafe impl<A, B, C> Send for PhantomSendParams<A, B, C> {}
unsafe impl<A, B, C> Sync for PhantomSendParams<A, B, C> {}

pub struct ServiceWrapper<Req: Send, Resp: Send, Handler: Service<Req, Resp>> {
    /// the handler that responds to messages
    //handler: Mutex<Option<Handler>>,

    /// This is intentionally not an AtomicBool,
    /// since we want to be able to use a condvar with it
    ///
    /// We send an option of a RequestParcel so we can, on shutdown,
    /// simply send a None through and break any wait loop
    started: Mutex<Option<Sender<Option<RequestParcel<Req, Resp>>>>>,

    /// Wait for a notify on this to know when the service has started
    notify_started: Condvar,

    // Contains (should, has) shut down
    shutdown: Mutex<(bool, bool)>,

    /// Wait for a notify on this to know when the service has fully shut down
    notify_shutdown: Condvar,

    //recv: Mutex<Option<Receiver<RequestParcel<Req, Resp>>>>,
    //send: OnceCell<Sender<RequestParcel<Req, Resp>>>,
    _pd: PhantomSendParams<Req, Resp, Handler>,
}

impl<Req: Send, Resp: Send, Handler: Service<Req, Resp>> ServiceWrapper<Req, Resp, Handler> {
    pub fn new() -> Self {
        // don't include channel pair yet, this means that every attempt
        // to send() will error out with a helpful error message
        Self {
            started: Mutex::new(None),
            notify_started: Condvar::new(),
            notify_shutdown: Condvar::new(),
            shutdown: Mutex::new((false, false)),
            _pd: unsafe { PhantomSendParams::new() },
        }
    }

    /// Tells service to shut down, returns a join handle that
    /// will exit once shutdown has completed
    pub fn shut_down(&'static self) -> JoinHandle<()> {
        std::thread::spawn(move || {
            let mut sh = self.shutdown.lock().unwrap();

            self.send_inner(None); // wake up from the wait loop to re-check shutdown notify

            sh.0 = true;

            while !sh.1 {
                sh = self.notify_shutdown.wait(sh).unwrap();
            }
        })
    }

    pub fn ask(&self, r: Req) -> Resp {
        self.send_inner(Some(r)).unwrap()
    }

    /// Panics if service is not yet started
    fn send_inner(&self, r: Option<Req>) -> Option<Resp> {
        let mut sender = self.started.lock().unwrap();

        let sender = loop {
            match sender.as_ref().cloned() {
                Some(v) => {
                    // service is started
                    break v;
                }
                None => {
                    // service not yet started, wait condvar
                    //self.notify_started.let cv = std::sync::Condvar::new();

                    tracing::info!(
                        "Service {} has not yet started, so waiting on the start condvar for it",
                        Handler::service_name()
                    );

                    sender = self.notify_started.wait(sender).unwrap();

                    continue;
                }
            }
        };

        let msg = format!(
            "Unexpected channel error when trying to send to service {}",
            Handler::service_name()
        );

        let (back_s, back_r) = crossbeam_channel::unbounded();

        let res = if let Some(v) = r {
            let parcel = RequestParcel {
                request: v,
                respond: back_s,
            };
            sender.send(Some(parcel)).expect(msg.as_str());

            std::mem::drop(sender); // make sure to lose the lock once we've sent

            let msg = format!(
                "Unexpected receive channel error when waiting for message back from {}",
                Handler::service_name()
            );
            let resp = back_r.recv().expect(msg.as_str());

            Some(resp)
        } else {
            sender
                .send(None)
                .expect("couldn't send an empty message to service");

            None
        };

        res
    }

    /// Panics if service was already started
    pub fn start(&'static self) {
        //let pv = self.started.swap(true, std::sync::atomic::Ordering::SeqCst);
        let mut sender = self.started.lock().unwrap();

        if sender.is_some() {
            panic!(
                "Service {} was already started, but tried to start it again",
                Handler::service_name()
            );
        }

        let (s, r) = crossbeam_channel::unbounded();

        *sender = Some(s); // store into lock

        self.notify_started.notify_all(); // wake up anything that *was* waiting on it

        // drop lock to unblock waiting senders
        std::mem::drop(sender);

        // we are the one, true, starter, so spawn a handle() loop
        std::thread::spawn(move || {
            let mut handler = Handler::init();

            while self.shutdown.lock().unwrap().0 == false {
                let req = r.recv().ok().flatten();

                if let Some(v) = req {
                    let RequestParcel { request, respond } = v;

                    let response = handler.handle(request);

                    match respond.send(response) {
                        Ok(_) => continue,
                        Err(e) => {
                            tracing::info!(
                                "Failed to send a response to a request that came to service {}",
                                Handler::service_name()
                            );
                        }
                    }
                }
            }

            handler.shut_down();

            self.shutdown.lock().unwrap().1 = true; // we have shutdown
        });
    }
}

pub struct RequestParcel<Req, Resp> {
    request: Req,
    respond: Sender<Resp>,
}

pub trait Service<Req, Resp> {
    /// Constructor for the service
    fn init() -> Self;

    fn service_name() -> String;

    /// Should only panic in extreme cases,
    /// panic will cause shutdown of application
    fn handle(&mut self, m: Req) -> Resp;

    /// Service should clean up anything that it needs to
    /// and exit gracefully, even in the face of errors!
    fn shut_down(self);
}

/*pub fn get_env_var(var: &str) -> String {
    dotenv().ok();
    let env_var = env::var(var)
        .map_err(|e| tracing::info!("could not find env var, var asked for was {var}, err was {e}"))
        .unwrap();
    return env_var;
}*/
