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

use std::fs::OpenOptions;
use std::fs::create_dir_all;
use std::fs::read_to_string;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use thiserror::Error;

#[allow(unused)]
use tracing::{debug, error, info, trace};

use super::GenId;

#[derive(Error, Debug)]
pub enum FrrErr {
    #[error("Failed to write config file: {0}")]
    COnfigFileWriteFailed(String),
    #[error("Failed to spawn reloader: {0}")]
    CmdSpawnFailed(String),
    #[error("Failed to wait for reloader: {0}")]
    CmdWaitFailed(String),
    #[error("Reloading error")]
    ReloadErr,
    #[error("Internal failure: {0}")]
    Failure(&'static str),
}

fn execute(
    reloader: &str,
    reload_args: &Vec<&str>,
    conf_file: &Path,
    test: bool,
) -> Result<(), FrrErr> {
    let mut args = if test {
        vec!["--test"]
    } else {
        vec!["--reload"]
    };
    args.extend_from_slice(reload_args);

    /* convert config file path back to string */
    let conf_file = conf_file.to_str().ok_or(FrrErr::Failure("Bad filename"))?;

    /* Build command */
    let mut cmd = Command::new(reloader);
    cmd.args(args.clone());
    cmd.arg(conf_file);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    debug!("Executing: {reloader} {} {}", args.join(" "), conf_file);

    /* execute */
    let output = cmd
        .spawn()
        .map_err(|e| {
            error!("Cmd spawn failed: {e}");
            FrrErr::CmdSpawnFailed(format!("{e}"))
        })?
        .wait_with_output()
        .map_err(|e| {
            error!("Cmd wait failed: {e}");
            FrrErr::CmdWaitFailed(format!("{e}"))
        })?;

    debug!("Reload completed (test:{test})");
    if !output.status.success() {
        error!(">>>> FRR Reload failed! <<<<");
        error!("stderr: {}", String::from_utf8_lossy(&output.stderr));
        error!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        return Err(FrrErr::ReloadErr);
    }

    if test {
        debug!("Successfully TESTED new configuration");
    } else {
        info!("Successfully APPLIED new configuration");
    }
    Ok(())
}

fn write_config_file(genid: GenId, config: &str, outdir: &str) -> Result<PathBuf, FrrErr> {
    /* file name to write the config into */
    let mut conf_file = PathBuf::from(outdir);
    conf_file.push(format!("frr-config-gen-{genid}"));
    conf_file.set_extension("conf");

    if let Some(parent) = conf_file.parent() {
        create_dir_all(parent)
            .map_err(|e| FrrErr::COnfigFileWriteFailed(format!("Could not create dir: {e:?}")))?;
    }

    /* create the file */
    let mut file = OpenOptions::new()
        .write(true)
        .read(true)
        .truncate(true)
        .create(true)
        .open(&conf_file)
        .map_err(|e| FrrErr::COnfigFileWriteFailed(format!("Unable to create file: {e:?}")))?;

    debug!("Successfully created config file at {conf_file:?}");

    /* write config to file */
    file.write_all(config.as_bytes()).map_err(|e| {
        FrrErr::COnfigFileWriteFailed(format!("Unable to write config file: {e:?}"))
    })?;

    /* read file back: fixme, this may not be needed */
    let contents = read_to_string(&conf_file).map_err(|e| {
        FrrErr::COnfigFileWriteFailed(format!("Unable to read written file: {e:?}"))
    })?;
    debug!("Requested config is:\n{contents}");

    Ok(conf_file)
}

fn do_frr_reload(
    reloader: &str,
    genid: GenId,
    config: &str,
    outdir: &str,
    reload_args: &Vec<&str>,
) -> Result<(), FrrErr> {
    let config_file = write_config_file(genid, config, outdir)?;

    // call frr-reload with --test
    execute(reloader, reload_args, &config_file, true)?;

    // call with --reload
    execute(reloader, reload_args, &config_file, false)?;
    Ok(())
}

pub fn frr_reload(
    reloader: &str,
    genid: GenId,
    config: &str,
    outdir: &str,
    reload_args: &Vec<&str>,
) -> String {
    match do_frr_reload(reloader, genid, config, outdir, reload_args) {
        Ok(()) => "Ok".to_string(),
        Err(e) => e.to_string(),
    }
}
