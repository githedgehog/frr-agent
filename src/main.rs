// SPDX-License-Identifier: Apache-2.0
// Copyright Open Network Fabric Authors

// An FRR config reloader daemon

use clap::Parser;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::SocketAddr;
use std::os::unix::net::UnixDatagram;
use std::path::Path;
use std::process::exit;
use tracing::{Level, debug, error};

use crate::reload::frr_reload;

mod reload;

// initialize logging
fn init_logging() {
    tracing_subscriber::fmt()
        .with_level(true)
        .with_max_level(Level::DEBUG)
        .compact()
        .init();
}

// open unix sock and bind it to the given address
fn open_unix_sock<P: AsRef<Path>>(bind_addr: &P) -> Result<UnixDatagram, &'static str> {
    let _ = std::fs::remove_file(bind_addr);
    let sock = UnixDatagram::bind(bind_addr).map_err(|_| "Failed to bind socket")?;
    let mut perms = fs::metadata(bind_addr)
        .map_err(|_| "Failed to retrieve path metadata")?
        .permissions();
    perms.set_mode(0o777);
    fs::set_permissions(bind_addr, perms).map_err(|_| "Failure setting permissions")?;
    sock.set_nonblocking(false)
        .map_err(|_| "Failed to set non-blocking")?;
    Ok(sock)
}

// wait for a request to come and return it as a String. We don't need to pass metadata
// as it can come in the config directly.
fn receive_request(sock: &UnixDatagram) -> Option<(SocketAddr, String)> {
    debug!("Waiting for a request ...");
    let mut rx_buff = vec![0u8; 1024];
    let mut msg_size_wire = [0u8; 8];
    let msg_size: u64;

    if let Err(e) = sock.recv_from(msg_size_wire.as_mut()) {
        error!("Error receiving msg size: {e}");
        return None;
    } else {
        msg_size = u64::from_ne_bytes(msg_size_wire);
        debug!("Received config size: {} octets", msg_size);
        if msg_size as usize > rx_buff.capacity() {
            rx_buff.resize(msg_size as usize, 0);
        }
    }
    debug!("Waiting to receive config of {} octets", msg_size);
    match sock.recv_from(rx_buff.as_mut_slice()) {
        Ok((rx_len, peer)) => {
            if let Ok(decoded) = String::from_utf8(rx_buff[0..rx_len].to_vec()) {
                Some((peer, decoded.to_owned()))
            } else {
                error!("Failed to decode config");
                None
            }
        }
        Err(e) => {
            error!("Failed to recv request: {e}");
            None
        }
    }
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
        self.confdir
            .as_ref()
            .map_or("/hedgehog/frr-reload.py", |v| v)
    }
    pub fn outdir(&self) -> &str {
        self.outdir.as_ref().map_or("/tmp/configs/hedgehog", |v| v)
    }
}

fn main() {
    let args = Args::parse();
    init_logging();

    // open & bind listening socket
    let sock_addr = &args.sock_path;
    let Ok(sock) = open_unix_sock(sock_addr) else {
        error!("FATAL: Failed to open unix socket. Exiting....");
        exit(1);
    };

    // build args for frr-reload from cmd line as a vector
    let frr_reload_args = build_reload_args(&args);

    debug!("frr-agent listening at '{sock_addr}' started");
    debug!("frr-agent writes configs at '{}'", &args.outdir());
    debug!("frr-agent reloader is '{}'", &args.reloader());

    loop {
        // receive request to apply config. Request is a string with the whole config
        if let Some((requestor, conf_string)) = receive_request(&sock) {
            // apply config
            let response = frr_reload(
                args.reloader(),
                &conf_string,
                args.outdir(),
                &frr_reload_args,
            );
            // reply right after
            if let Err(e) = sock.send_to_addr(response.as_bytes(), &requestor) {
                error!("Error sending response: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::frr_reload;
    use tracing::debug;
    use tracing_test::traced_test;
    const SAMPLE_CONFIG: &str = "
!
log stdout
frr defaults datacenter
hostname FOO
service integrated-vtysh-config
!
router bgp 65000
bgp router-id 7.0.0.100
no bgp network import-check
no bgp ebgp-requires-policy
no bgp default ipv4-unicast
neighbor 7.0.0.2 remote-as 65000
neighbor 7.0.0.2 capability dynamic
neighbor 7.0.0.2 description Spine switch
neighbor 7.0.0.2 update-source 7.0.0.100
!
address-family l2vpn evpn
neighbor 7.0.0.2 activate
advertise-all-vni
exit-address-family
!
exit
";

    // This is not really a test and is unfinished
    #[test]
    #[traced_test]
    fn test_reloader() {
        let config_string = SAMPLE_CONFIG;
        debug!("Got config!");

        let reloader = "frr-reload.py";
        let outdir = "/tmp/configs/hedgehog";

        let args = vec![
            "--stdout",
            "--debug",
            "--confdir",
            "/tmp",
            "--bindir",
            "/usr/local/bin",
        ];

        let result = frr_reload(reloader, &config_string, outdir, &args);
        println!("result: {result}");
    }
}
