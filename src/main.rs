// SPDX-License-Identifier: Apache-2.0
// Copyright Open Network Fabric Authors

// An FRR config reloader daemon

#![deny(
    unsafe_code,
    clippy::all,
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use bytes::BytesMut;
use clap::Parser;

use signal_hook::consts::{SIGINT, SIGQUIT, SIGTERM};
use signal_hook::iterator::Signals;

use std::fs;
use std::io::Read;
use std::io::Write;
use std::net::Shutdown;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};

use std::path::Path;
use std::process::exit;
use std::str;
use std::str::FromStr;
use std::thread;
use std::thread::sleep;
use std::time::Duration;
#[allow(unused)]
use tracing::{Level, debug, error, info, warn};

use crate::reload::frr_reload;

mod reload;
pub type GenId = i64;

// initialize logging
fn init_logging(loglevel: Level) {
    tracing_subscriber::fmt()
        .with_level(true)
        .with_max_level(loglevel)
        .compact()
        .init();
}

fn create_unix_listener(bind_addr: &str) -> Result<UnixListener, String> {
    // clean up entry in file system
    let _ = std::fs::remove_file(bind_addr);
    let bind_path = Path::new(bind_addr);

    // create intermediate directories if needed
    if let Some(parent_dir) = bind_path.parent() {
        fs::create_dir_all(parent_dir).map_err(|e| format!("Could not create sock paths: {e}"))?;
    }

    // build listener and bind it
    let listener = UnixListener::bind(bind_addr).map_err(|e| format!("failed to bind: {e}"))?;
    listener
        .set_nonblocking(false)
        .map_err(|e| format!("Failed to set blocking: {e}"))?;

    // grant permissions -- FIXME, we may want this to be more strict
    let mut perms = fs::metadata(bind_addr)
        .map_err(|_| "Failed to retrieve path metadata".to_string())?
        .permissions();
    perms.set_mode(0o777);
    fs::set_permissions(bind_addr, perms).map_err(|_| "Failure setting permissions")?;

    Ok(listener)
}

fn receive_request(sock: &mut UnixStream) -> Result<(GenId, String), String> {
    debug!("━━━━━━ Waiting for data ━━━━━━");

    let mut len_buf = [0u8; 8];
    let mut genid_buf = [0u8; 8];

    sock.read_exact(&mut len_buf)
        .map_err(|e| format!("Could not receive msg-len: {e}"))?;
    sock.read_exact(&mut genid_buf)
        .map_err(|e| format!("Could not receive genid: {e}"))?;

    let msg_size = usize::try_from(u64::from_ne_bytes(len_buf))
        .map_err(|e| format!("Could not determine message length: {e}"))?;
    let genid = i64::from_ne_bytes(genid_buf);

    let mut rx_buff = vec![0u8; msg_size];
    sock.read_exact(&mut rx_buff)
        .map_err(|e| format!("Could not receive request body: {e}"))?;
    let request = String::from_utf8(rx_buff[0..msg_size].to_vec())
        .map_err(|e| format!("Could not decode request body: {e:?}"))?;

    debug!("Successfully received request. data-len: {msg_size} octets genid:{genid}");
    Ok((genid, request))
}

fn send_response(sock: &mut UnixStream, genid: GenId, msg: &[u8]) -> Result<(), String> {
    /* length of data */
    let length = msg.len() as u64;

    /* assemble wire message: |length|genid|data| */
    let mut wire_msg = BytesMut::with_capacity(msg.len() + 16);
    wire_msg.extend_from_slice(&length.to_ne_bytes());
    wire_msg.extend_from_slice(&genid.to_ne_bytes());
    wire_msg.extend_from_slice(msg);

    /* send wire message */
    sock.write_all(&wire_msg)
        .map_err(|e| format!("Failed to send message: {e}"))?;
    debug!("Successfully sent msg. data-len: {length} genid: {genid}");
    Ok(())
}

// build frr-reload args from cmd line. If some params are not specified, we provide our own defaults here
// so that we can exactly log what parameters were passed (even if frr-reload has its own defaults)
fn build_reload_args(args: &Args) -> Vec<&str> {
    vec![
        "--stdout",
        "--debug",
        "--bindir",
        args.binddir(),
        "--rundir",
        args.rundir(),
        "--confdir",
        args.confdir(),
    ]
}

// cmd line args the reloader accepts. Fixme: use PathBuf instead of String?
#[derive(Parser)]
#[command(name = "FRR reload agent")]
#[command(version = "1.0")]
#[command(about = "Daemon to reload FRR configs", long_about = None)]
pub(crate) struct Args {
    // mandatory
    #[arg(long, value_name = "Unix socket bind path")]
    sock_path: String,

    // optional
    #[arg(
        long,
        value_name = "Loglevel (error, warn, info, debug, trace). Defaults to debug"
    )]
    loglevel: Option<String>,
    #[arg(long, value_name = "Directory where received configs are stored")]
    outdir: Option<String>,
    #[arg(long, value_name = "Full path to reloader (frr-reload.bin|py)")]
    reloader: Option<String>,
    #[arg(long, value_name = "Directory of vtysh")]
    bindir: Option<String>,
    #[arg(long, value_name = "Directory of where frr-reload writes temp files")]
    rundir: Option<String>,
    #[arg(long, value_name = "Directory of frr config files")]
    confdir: Option<String>,
    #[arg(long, value_name = "vtysh sock (UNUSED atm)")]
    vtysock: Option<String>,

    // testing-only
    #[arg(long)]
    always_ok: bool,
    #[arg(
        long,
        value_name = "Artificially increase processing time by this number of seconds"
    )]
    proc_time: Option<u64>,
}
impl Args {
    pub fn binddir(&self) -> &str {
        self.bindir.as_ref().map_or("/usr/local/bin", |v| v)
    }
    pub fn rundir(&self) -> &str {
        self.rundir.as_ref().map_or("/var/run/frr", |v| v)
    }
    pub fn confdir(&self) -> &str {
        self.confdir.as_ref().map_or("/etc/frr", |v| v)
    }
    pub fn reloader(&self) -> &str {
        self.reloader
            .as_ref()
            .map_or("/hedgehog/frr-reload.py", |v| v)
    }
    pub fn outdir(&self) -> &str {
        self.outdir.as_ref().map_or("/tmp/configs/hedgehog", |v| v)
    }
    pub fn loglevel(&self) -> Result<Level, ()> {
        if let Some(loglevel) = &self.loglevel {
            Level::from_str(loglevel.as_ref()).map_err(|_| ())
        } else {
            Ok(Level::DEBUG)
        }
    }
    pub fn proc_time(&self) {
        if let Some(time) = self.proc_time {
            debug!("Sleeping for {time} seconds...");
            sleep(Duration::from_secs(time));
        }
    }
}

fn main() {
    let args = Args::parse();
    let Ok(loglevel) = args.loglevel() else {
        println!("Bad loglevel");
        exit(1);
    };
    init_logging(loglevel);

    let bind_addr = args.sock_path.clone();
    if let Ok(mut signals) = Signals::new([SIGINT, SIGQUIT, SIGTERM]) {
        thread::spawn(move || {
            if let Some(sig) = signals.forever().next() {
                match sig {
                    SIGINT | SIGTERM | SIGQUIT => {
                        warn!("Terminated (pid {})", std::process::id());
                        if std::fs::remove_file(bind_addr.clone()).is_ok() {
                            info!("Removed sock at {bind_addr}");
                        }
                        std::process::exit(0);
                    }
                    _ => {
                        warn!("Ignoring signal {sig}");
                    }
                }
            }
        });
    }

    debug!("Starting FRR-agent...");

    /* create unix sock stream listener */
    let bind_addr = &args.sock_path;
    let listener = match create_unix_listener(bind_addr) {
        Ok(listener) => listener,
        Err(e) => {
            error!("FATAL: Failed to open unix socket: {e:?}. Exiting....");
            exit(1);
        }
    };

    // build args for frr-reload from cmd line as a vector
    let frr_reload_args = build_reload_args(&args);

    debug!("frr-agent listening at '{bind_addr}' started");
    debug!("frr-agent writes configs at '{}'", &args.outdir());
    debug!("frr-agent reloader is '{}'", &args.reloader());
    debug!("frr-agent loglevel is '{}'", loglevel);

    loop {
        debug!("┣━━━━ Waiting for connection ━━━━━┫");
        if let Ok((mut stream, peer)) = listener.accept() {
            debug!("Got connection from {peer:?}");
            loop {
                let Ok((genid, request)) = receive_request(&mut stream) else {
                    error!("An error occurred. Shutting down connection...");
                    let _ = stream.shutdown(Shutdown::Both);
                    break; /* move to accept again */
                };
                args.proc_time();
                let response = if &request == "KEEPALIVE" {
                    debug!("Got keepalive request from {peer:?}");
                    "Ok".to_string()
                } else if args.always_ok {
                    warn!("This agent is running in always-ok mode and will always report SUCCESS");
                    "Ok".to_string()
                } else {
                    debug!("Got config request from {peer:?} for generation {genid}");
                    frr_reload(
                        args.reloader(),
                        genid,
                        &request,
                        args.outdir(),
                        &frr_reload_args,
                    )
                };
                if let Err(e) = send_response(&mut stream, genid, response.as_bytes()) {
                    error!("Error sending response: {e:?}. Shutting down connection...");
                    let _ = stream.shutdown(Shutdown::Both);
                    break; /* move to accept again */
                }
            }
        }
    }
}
