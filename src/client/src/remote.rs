use std::{
    io::{stdout, Write},
    mem::swap,
    panic::{AssertUnwindSafe, BacktraceStyle, PanicInfo},
    sync::Arc,
};

use std::net::Ipv4Addr;
use tokio::net::{TcpListener, TcpStream};

use remoc::prelude::*;

use common::prelude::{
    anyhow,
    chrono::NaiveDate,
    config::{CliConfig, HostPortPair},
    futures::FutureExt,
    inquire::{self, validator::StringValidator},
    itertools::Itertools,
    parking_lot::Mutex,
    rand,
    tokio::{self, sync::mpsc as tokio_mpsc},
    tracing,
};

use serde::{Deserialize, Serialize};
use tascii::{
    executors::{self, spawn_on_tascii_tokio_options, RtOptions},
    prelude::Runtime,
    set_local_hook,
};

use crate::{cli_entry, LiblaasStateInstruction};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SentObject {
    display_as: String,
    index: Option<usize>,
    serialized: Option<String>,
}

impl std::fmt::Display for SentObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.display_as.fmt(f)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ToServer {
    Selection(SentObject),
    MultiSelection(Vec<SentObject>),
    Text(String),
    Date(NaiveDate),
    Error(String),
    Confirm(bool),
    Terminate(),
}

#[derive(Serialize, Deserialize, Clone)]
pub enum ToClient {
    Print(Vec<u8>),
    Query(ToClientQuery),
    Terminate(),
}

impl std::fmt::Debug for ToClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Query(q) => {
                write!(f, "{q:?}")
            }
            Self::Terminate() => {
                write!(f, "Termination Request")
            }
            Self::Print(p) => match String::from_utf8(p.clone()) {
                Ok(v) => {
                    writeln!(f, "Print({v})")
                }
                Err(_) => {
                    writeln!(f, "{p:?}")
                }
            },
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToClientQuery {
    message: String,
    inner: ToClientQueryInner,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ToClientQueryInner {
    Select {
        options: Vec<SentObject>,
        help_message: Option<String>,
    },
    MultiSelect {
        options: Vec<SentObject>,
        help_message: Option<String>,
    },
    Text {
        help_message: Option<String>,
    },
    Editor {},
    Password {
        help_message: Option<String>,
    },
    DateSelect {
        min_date: Option<NaiveDate>,
        max_date: Option<NaiveDate>,
        start_date: Option<NaiveDate>,
    },
    Confirm {
        default: Option<bool>,
    },
}

pub struct SessionClient {
    outgoing_channel: rch::mpsc::Sender<ToServer>,
    incoming_channel: Mutex<rch::mpsc::Receiver<ToClient>>,
}

pub struct SessionServer {
    outgoing_channel: rch::mpsc::Sender<ToClient>,
    incoming_channel: Mutex<rch::mpsc::Receiver<ToServer>>,
}

impl SessionClient {
    pub fn send(&self, v: ToServer) {
        let sref: &'static Self = unsafe { std::mem::transmute(self) };

        executors::spawn_on_tascii_tokio("cli", async {
            let _ = sref
                .outgoing_channel
                .send(v)
                .await
                .map_err(|e| println!("Couldn't send message to server got: {e:?}"));
            //println!("Sent outgoing");
        });
    }

    pub fn recv(&self) -> Result<ToClient, anyhow::Error> {
        let sref: &'static Self = unsafe { std::mem::transmute(self) };

        let msg = executors::spawn_on_tascii_tokio("cli", async {
            tracing::info!("Locking channel");
            let mut g = sref.incoming_channel.lock();
            tracing::info!("Locked channel");
            let msg = g.recv().await;

            tracing::info!("Got msg: {msg:?}");

            msg
        });

        match msg {
            Ok(None) => Err(anyhow::Error::msg("no msg present")),
            Ok(Some(v)) => Ok(v),
            Err(e) => Err(e)?,
        }
    }
}

impl SessionServer {
    pub fn send(&self, v: ToClient) {
        let sref: &'static Self = unsafe { std::mem::transmute(self) };

        executors::spawn_on_tascii_tokio("cli", async move {
            sref.outgoing_channel
                .send(v)
                .await
                .expect("Couldn't send msg");
        });
    }

    pub fn recv(&self) -> Result<ToServer, anyhow::Error> {
        let sref: &'static Self = unsafe { std::mem::transmute(self) };

        let msg = executors::spawn_on_tascii_tokio("cli", async {
            tracing::info!("Locking channel");
            let mut g = sref.incoming_channel.lock();
            tracing::info!("Locked channel");
            let msg = g.recv().await;

            tracing::info!("Got msg: {msg:?}");

            msg
        });

        match msg {
            Ok(None) => Err(anyhow::Error::msg("no msg present")),
            Ok(Some(v)) => Ok(v),
            Err(e) => Err(e)?,
        }
    }
}

pub struct Select<T> {
    message: String,
    help_message: Option<String>,
    options: Vec<T>,
}

impl<T> Select<T>
where
    T: std::fmt::Display + Clone,
{
    pub fn new(message: &str, options: Vec<T>) -> Self {
        Self {
            message: message.to_owned(),
            help_message: None,
            options,
        }
    }

    pub fn with_help_message<I: Into<String>>(self, msg: I) -> Self {
        Self {
            help_message: Some(msg.into()),
            ..self
        }
    }

    pub fn prompt(self, within: &Server) -> Result<T, anyhow::Error> {
        within.session.send(ToClient::Query(ToClientQuery {
            message: self.message,
            inner: ToClientQueryInner::Select {
                help_message: self.help_message,
                options: self
                    .options
                    .iter()
                    .enumerate()
                    .map(|(idx, v)| SentObject {
                        display_as: format!("{v}"),
                        index: Some(idx),
                        serialized: None,
                    })
                    .collect_vec(),
            },
        }));

        tracing::info!("Waiting for reply to query");

        match within.session.recv()? {
            ToServer::Selection(v) => {
                let idx = v.index.ok_or(anyhow::Error::msg(
                    "user did not send back an indexing choice",
                ))?;
                tracing::warn!("Got select idx: {idx}");
                let v = self
                    .options
                    .get(idx)
                    .ok_or(anyhow::Error::msg("user passed index out of bounds"))?
                    .clone();

                Ok(v)
            }
            other => {
                tracing::warn!("Bad ToServer: {other:?}");
                Err(anyhow::Error::msg(format!(
                    "user sent back a message that wasn't a reply to select, got: {other:?}"
                )))
            }
        }
    }
}

pub struct Text {
    message: String,
    help_message: Option<String>,
    validators: Vec<Box<dyn StringValidator>>,
}

impl Text {
    pub fn new(message: &str) -> Self {
        Self {
            validators: vec![],
            help_message: None,
            message: message.to_owned(),
        }
    }

    pub fn with_help_message<I: Into<String>>(self, msg: I) -> Self {
        Self {
            help_message: Some(msg.into()),
            ..self
        }
    }

    pub fn with_validator<V: StringValidator + 'static>(self, validator: V) -> Self {
        Self {
            validators: {
                let mut v = self.validators;
                v.push(Box::new(validator));

                v
            },
            ..self
        }
    }

    pub fn prompt(self, within: &Server) -> Result<String, anyhow::Error> {
        within.session.send(ToClient::Query(ToClientQuery {
            message: self.message,
            inner: ToClientQueryInner::Text {
                help_message: self.help_message,
            },
        }));

        match within.session.recv()? {
            ToServer::Text(v) => Ok(v),
            other => Err(anyhow::Error::msg(format!(
                "user sent back a message that wasn't a reply to text, got: {other:?}"
            ))),
        }
    }
}

pub struct Password {
    message: String,
    help_message: Option<String>,
    validators: Vec<Box<dyn StringValidator>>,
}

impl Password {
    pub fn new(message: &str) -> Self {
        Self {
            help_message: None,
            validators: vec![],
            message: message.to_owned(),
        }
    }

    pub fn with_validator<V: StringValidator + 'static>(self, validator: V) -> Self {
        Self {
            validators: {
                let mut v = self.validators;
                v.push(Box::new(validator));

                v
            },
            ..self
        }
    }

    pub fn prompt(self, within: &Server) -> Result<String, anyhow::Error> {
        within.session.send(ToClient::Query(ToClientQuery {
            message: self.message,
            inner: ToClientQueryInner::Password {
                help_message: self.help_message,
            },
        }));

        match within.session.recv()? {
            ToServer::Text(v) => Ok(v),
            other => Err(anyhow::Error::msg(format!(
                "user sent back a message that wasn't a reply to text, got: {other:?}"
            ))),
        }
    }
}

impl ToClientQuery {
    pub fn prompt_here(self) -> Vec<ToServer> {
        match self.inner {
            ToClientQueryInner::Select {
                options,
                help_message,
            } => {
                let res = inquire::Select::new(&self.message, options);

                let res = match help_message.as_ref() {
                    None => res,
                    Some(hm) => res.with_help_message(&hm),
                };

                let res = res.prompt();

                match res {
                    Ok(so) => vec![ToServer::Selection(so)],
                    Err(e) => vec![ToServer::Error(format!(
                        "error getting selection from user: {e:?}",
                    ))],
                }
            }
            ToClientQueryInner::MultiSelect {
                options,
                help_message,
            } => {
                let res = inquire::MultiSelect::new(&self.message, options);

                let res = match help_message.as_ref() {
                    None => res,
                    Some(hm) => res.with_help_message(&hm),
                };

                let res = res.prompt();

                match res {
                    Ok(so) => vec![ToServer::MultiSelection(so)],
                    Err(e) => vec![ToServer::Error(format!(
                        "error getting selection from user: {e:?}",
                    ))],
                }
            }
            ToClientQueryInner::Text { help_message } => {
                let res = inquire::Text::new(&self.message);

                let res = match help_message.as_ref() {
                    None => res,
                    Some(hm) => res.with_help_message(&hm),
                };

                let res = res.prompt();

                match res {
                    Ok(t) => vec![ToServer::Text(t)],
                    Err(e) => vec![ToServer::Error(format!(
                        "error getting text from user: {e:?}",
                    ))],
                }
            }
            ToClientQueryInner::Editor {} => todo!(),
            ToClientQueryInner::Password { help_message } => {
                let res = inquire::Text::new(&self.message);

                let res = match help_message.as_ref() {
                    None => res,
                    Some(hm) => res.with_help_message(&hm),
                };

                let res = res.prompt();

                match res {
                    Ok(t) => vec![ToServer::Text(t)],
                    Err(e) => vec![ToServer::Error(format!(
                        "error getting password from user: {e:?}",
                    ))],
                }
            }
            ToClientQueryInner::DateSelect {
                min_date,
                max_date,
                start_date,
            } => {
                let mut res = inquire::DateSelect::new(&self.message);

                if let Some(md) = min_date {
                    res = res.with_min_date(md);
                }

                if let Some(md) = max_date {
                    res = res.with_max_date(md);
                }

                if let Some(sd) = start_date {
                    res = res.with_starting_date(sd);
                }

                let res = res.prompt();

                match res {
                    Ok(d) => vec![ToServer::Date(d)],
                    Err(e) => vec![ToServer::Error(format!(
                        "error getting date from user: {e:?}",
                    ))],
                }
            }
            ToClientQueryInner::Confirm { default } => {
                let res = inquire::Confirm::new(&self.message);

                let res = match default {
                    None => res,
                    Some(d) => res.with_default(d),
                };

                let res = res.prompt();

                match res {
                    Ok(b) => vec![ToServer::Confirm(b)],
                    Err(e) => vec![ToServer::Error(format!(
                        "error getting confirm from user: {e:?}",
                    ))],
                }
            }
        }
    }
}

pub struct Client {
    session: SessionClient,
}

impl Client {
    async fn run(&mut self) {
        loop {
            let m = self.session.recv();

            let m = match m {
                Ok(m) => m,
                Err(e) => {
                    println!("Error, no message received, result: {e:?}");
                    return;
                }
            };

            match m {
                ToClient::Terminate() => {
                    println!("Session terminated");
                    return;
                }
                ToClient::Print(msg) => {
                    let _ = stdout().write_all(&msg[..]);
                }
                ToClient::Query(q) => {
                    let vals = q.prompt_here();

                    //println!("Sending msg from client..");

                    for msg in vals {
                        self.session.send(msg);
                    }
                }
            }
        }
    }
}

pub struct Server {
    session: SessionServer,
    outgoing_buffer: Mutex<Vec<u8>>,
}

pub type PortIdent = u16;

impl Server {
    pub fn new_session() -> Result<(Server, PortIdent), anyhow::Error> {
        todo!()
    }

    pub fn run(self) {}
}

impl std::io::Write for &Server {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let output: std::io::Result<usize> = self.outgoing_buffer.lock().write(buf);
        self.flush();
        output
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut g = self.outgoing_buffer.lock();
        g.flush()?;

        let mut r = Vec::new();

        swap(&mut *g, &mut r);

        self.session.send(ToClient::Print(r));

        Ok(())
    }
}

pub async fn start_session() -> PortIdent {
    let cfg = common::prelude::config::settings().cli.clone();

    let socket = TcpStream::connect((cfg.external_url.host.clone(), cfg.external_url.port))
        .await
        .unwrap();
    let (socket_rx, socket_tx) = socket.into_split();

    println!("Connected stream for handshake");

    let (conn, mut tx, _rx): (_, _, rch::base::Receiver<()>) =
        remoc::Connect::io(remoc::Cfg::default(), socket_rx, socket_tx)
            .await
            .unwrap();

    tokio::spawn(conn);

    //tokio::time::sleep(Duration::from_millis(200)).await;

    println!("Connected handshake channel");

    let (init_tx, mut init_rx) = rch::mpsc::channel(1);

    println!("Made init channel");

    //tokio::time::sleep(Duration::from_millis(200)).await;

    tx.send(InitiationRequest { reply_to: init_tx })
        .await
        .unwrap();

    println!("Sent init request...");

    let InitiationReply { use_port } = init_rx
        .recv()
        .await
        .unwrap()
        .expect("no reply to init request");

    use_port
}

pub async fn cli_client_entry() {
    let cfg = common::prelude::config::settings().cli.clone();

    let port = start_session().await;

    println!("Got init reply, moving port to {port}");

    let socket = TcpStream::connect((cfg.external_url.host.clone(), port))
        .await
        .unwrap();
    let (socket_rx, socket_tx) = socket.into_split();

    println!("Bound new sock");

    let (conn, mut tx, _rx): (_, _, rch::base::Receiver<()>) =
        remoc::Connect::io(remoc::Cfg::default(), socket_rx, socket_tx)
            .await
            .unwrap();

    tokio::spawn(conn);

    let (to_client, from_server) = rch::mpsc::channel(20);

    let (setup_tx, mut setup_rx) = rch::mpsc::channel(1);

    println!("Sending setup request...");

    tx.send(SetupRequest {
        reply_to: setup_tx,
        using_send: to_client,
    })
    .await
    .unwrap();

    println!("Waiting for setup reply...");

    let SetupReply { using_send } = setup_rx
        .recv()
        .await
        .unwrap()
        .expect("no reply to init request");

    println!("Setup complete, starting CLI...");

    // now bind to the actual port for the session

    cli_client(using_send, from_server).await;
}

pub async fn cli_client(tx: rch::mpsc::Sender<ToServer>, rx: rch::mpsc::Receiver<ToClient>) {
    let mut client = Client {
        session: SessionClient {
            outgoing_channel: tx,
            incoming_channel: Mutex::new(rx),
        },
    };

    client.run().await;
}

#[derive(Debug, Clone)]
pub enum CliServerError {
    InitiationReceiveFailed,
    BindFailed(String),
    ConfigError(String),
    UnknownError(String),
}

impl<E> From<E> for CliServerError
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn from(value: E) -> Self {
        CliServerError::UnknownError(format!("{value:?}"))
    }
}

async fn spawn_cli_handler(
    tascii_rt: &'static Runtime,
    liblaas_tx: tokio_mpsc::Sender<LiblaasStateInstruction>,
) -> Result<PortIdent, CliServerError> {
    let CliConfig {
        bind_addr,
        external_url: _,
    } = common::prelude::config::settings().cli.clone();

    let HostPortPair { host, port: _ } = bind_addr;

    tracing::info!("Going to bind session port");
    let listener = TcpListener::bind((host.as_str(), 0)).await.unwrap();

    let local_addr = listener.local_addr()?;

    let port = local_addr.port();

    tracing::info!("Bound session port as port {port}");

    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let (socket_rx, socket_tx) = socket.into_split();

        let (conn, _, mut rx): (_, rch::base::Sender<()>, rch::base::Receiver<SetupRequest>) =
            remoc::Connect::io(remoc::Cfg::default(), socket_rx, socket_tx)
                .await
                .unwrap();

        tokio::spawn(conn);

        tracing::info!("New setup session");

        let SetupRequest {
            reply_to,
            using_send,
        } = rx
            .recv()
            .await
            .expect("no request sent by cli")
            .expect("empty request?");

        tracing::info!("Setup request received");

        let (to_server, from_client) = rch::mpsc::channel(10);

        let server = Server {
            session: SessionServer {
                outgoing_channel: using_send,
                incoming_channel: Mutex::new(from_client),
            },
            outgoing_buffer: Mutex::default(),
        };

        tracing::info!("Made a CLI session server instance");

        let tx = liblaas_tx.clone();
        tokio::spawn(async move {
            let msg = cli_server(tascii_rt, server).await;
            let _sr = tx.try_send(msg).map_err(|e| {
                tracing::warn!("Couldn't send a handover token to client, error: {e:?}")
            });
        });

        tracing::info!("Spawned the cli server");

        //let port = socket.local_addr().unwrap().port();
        reply_to
            .send(SetupReply {
                using_send: to_server,
            })
            .await
            .unwrap();

        tracing::info!("Sent setup reply");
    });

    Ok(port)
}

pub async fn cli_server_entry(
    tascii_rt: &'static Runtime,
    liblaas_tx: tokio_mpsc::Sender<LiblaasStateInstruction>,
) -> Result<LiblaasStateInstruction, CliServerError> {
    tracing::info!("Entry for cli server");

    let CliConfig {
        bind_addr,
        external_url: _,
    } = common::prelude::config::settings().cli.clone();

    let HostPortPair { host, port } = bind_addr;

    loop {
        tracing::info!("Binding setup sock");
        let sock = tokio::net::TcpSocket::new_v4()?;
        sock.set_nodelay(true)?;
        sock.set_reuseaddr(true)?;
        sock.set_reuseport(true)?;

        let ipaddr: Ipv4Addr = host.parse()?;

        sock.bind((ipaddr, port).into())?;

        let listener = sock.listen(1)?;

        tracing::info!("Bound setup sock");

        let v = listener.accept().await;

        let v = match v {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("Couldn't accept for listener: {e:?}");
                continue;
            }
        };

        let (stream, _sock) = v;

        let peer_addr = stream.peer_addr().unwrap();
        tracing::info!("Got a setup request from {peer_addr:?}");

        let (socket_rx, socket_tx) = stream.into_split();
        let rcon = remoc::Connect::io(remoc::Cfg::compact(), socket_rx, socket_tx).await;

        let (_, mut rx): (
            rch::base::Sender<()>,
            rch::base::Receiver<InitiationRequest>,
        ) = match rcon {
            Ok((c, t, r)) => {
                tracing::info!("Bound an rch channel");
                tokio::spawn(c);
                (t, r)
            }
            Err(e) => {
                tracing::error!("Couldn't make channel: {e:?}");
                continue;
            }
        };

        let msg = rx.recv().await;

        let InitiationRequest { reply_to } = if let Ok(Some(v)) = msg {
            v
        } else {
            continue;
        };

        let port = spawn_cli_handler(tascii_rt, liblaas_tx.clone()).await;

        let port = match port {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("Couldn't spawn handler: {e:?}");
                continue;
            }
        };

        tracing::info!("Spawned the handler");

        let _ = reply_to
            .send(InitiationReply { use_port: port })
            .await
            .map_err(|e| {
                tracing::warn!("Failed to send InitiationReply to client: {e:?}");
            });
        tracing::info!("Sent the IR reply");
    }
}

thread_local! {
    static SERVER_STUB: Mutex<Option<Arc<Server>>> = Mutex::new(None);
}

pub async fn cli_server(tascii_rt: &'static Runtime, server: Server) -> LiblaasStateInstruction {
    let sr = Arc::new(server);

    fn panic_handler(info: &PanicInfo<'_>) {
        let style = if !info.can_unwind() {
            Some(BacktraceStyle::Full)
        } else {
            std::panic::get_backtrace_style()
        };

        // The current implementation always returns `Some`.
        let location = info.location();
        if let Some(location) = location {
            let msg = match info.payload().downcast_ref::<&'static str>() {
                Some(s) => *s,
                None => match info.payload().downcast_ref::<String>() {
                    Some(s) => &s[..],
                    None => "Box<dyn Any>",
                },
            };
            let thread = std::thread::current();
            let name = thread.name().unwrap_or("<unnamed>");

            let mut output = String::new();

            use std::fmt::Write;

            let _ = writeln!(
                &mut output,
                "thread '{name}' panicked at '{msg}', {location}"
            );

            match style {
                Some(BacktraceStyle::Short)
                | Some(BacktraceStyle::Full)
                | Some(BacktraceStyle::Off) => {
                    let bt = std::backtrace::Backtrace::force_capture().to_string();

                    let _ = writeln!(output, "{bt}");
                }
                Some(_) => {}
                None => {}
            }

            SERVER_STUB.with(|ss| {
                let g = ss.lock();

                if let Some(r) = g.as_ref() {
                    let res1 = writeln!(r.as_ref(), "Panic within runtime: {output}");
                    let res2 = r.as_ref().flush();

                    if let Err(e) = res1 {
                        tracing::error!("Error while sending a panic over the wire to CLI: {e:?}");
                    }

                    if let Err(e) = res2 {
                        tracing::error!("Error while sending a panic over the wire to CLI: {e:?}");
                    }
                } else {
                    tracing::error!("Tried to dispatch to a local CLI server, but none existed for current thread");
                }
            })

            //tracing::error!("Panic within runtime:\n{output}");
        }
    }

    let res = loop {
        let sr = sr.clone();
        let si = sr.clone();
        let cli_id: usize = rand::random::<usize>() % 10usize.pow(5);
        // TODO: cleanup this (small) leak

        let res = spawn_on_tascii_tokio_options(
            format!("cli_rt_{cli_id}"),
            async move {
                set_local_hook(Some(Box::new(panic_handler)));

                SERVER_STUB.with(|ss| {
                    *ss.lock() = Some(si.clone());
                });
                let res = AssertUnwindSafe(cli_entry(tascii_rt, si.as_ref()))
                    .catch_unwind()
                    .await;

                res.map_err(|_| ())
            },
            RtOptions { threads: Some(1) },
        );

        match res {
            Err(()) => {
                let _ = writeln!(sr.as_ref(), "Panic encountered within CLI, reprompting...");
                continue;
            }
            Ok(Err(e)) => {
                let _ = writeln!(sr.as_ref(), "Error encountered within CLI: {e:?}");
                continue;
            }
            Ok(Ok(v)) => match v {
                LiblaasStateInstruction::DoNothing() => continue,
                LiblaasStateInstruction::ShutDown() => {
                    let _ = writeln!(sr.as_ref(), "Shutdown instruction issued");
                    break Ok(v);
                }
                LiblaasStateInstruction::ExitCLI() => {
                    let _ = writeln!(sr.as_ref(), "Exiting CLI");
                    sr.session.send(ToClient::Terminate());
                    tracing::info!("Exiting CLI");
                    break Ok(v);
                }
            },
        }
    };

    SERVER_STUB.with(|ss| {
        *ss.lock() = None;
    });

    let _ = (sr.as_ref()).flush();

    match res {
        Ok(v) => v,
        Err(()) => LiblaasStateInstruction::DoNothing(),
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SetupRequest {
    reply_to: rch::mpsc::Sender<SetupReply>,

    using_send: rch::mpsc::Sender<ToClient>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SetupReply {
    using_send: rch::mpsc::Sender<ToServer>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct InitiationRequest {
    reply_to: rch::mpsc::Sender<InitiationReply>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct InitiationReply {
    use_port: PortIdent,
}
