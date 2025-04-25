// SPDX-License-Identifier: Apache-2.0
// Copyright Open Network Fabric Authors

// An FRR config reloader daemon

use std::fs::OpenOptions;
use std::fs::create_dir_all;
use std::fs::read_to_string;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use thiserror::Error;

#[allow(unused)]
use tracing::{debug, error, trace};

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
}

fn execute(reloader: &str, args: &Vec<&str>, conf_file: &Path) -> Result<(), FrrErr> {
    /* Build command */
    let mut cmd = Command::new(reloader);
    cmd.args(args);
    cmd.arg(conf_file.to_str().unwrap());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    debug!(
        "Executing: {reloader} {} {}",
        args.join(" "),
        conf_file.display()
    );

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

    if !output.status.success() {
        debug!("stderr: {}", String::from_utf8_lossy(&output.stderr));
        debug!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        error!(">>>> FRR Reload failed! <<<<");
        return Err(FrrErr::ReloadErr);
    }
    debug!("Successfully reloaded new configuration");
    Ok(())
}

fn write_config_file(genid: GenId, config: &str, outdir: &str) -> Result<PathBuf, FrrErr> {
    /* file name to write the config into */
    let mut conf_file = PathBuf::from(outdir);
    conf_file.push(format!("frr-config-gen-{genid}"));
    conf_file.set_extension("conf");
    create_dir_all(conf_file.parent().unwrap())
        .map_err(|e| FrrErr::COnfigFileWriteFailed(format!("Could not create dir: {e:?}")))?; // fixme unwrap

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
    let mut args = vec!["--test"];
    args.extend_from_slice(reload_args);
    execute(reloader, &args, &config_file)?;

    // call with --reload
    let mut args = vec!["--reload"];
    args.extend_from_slice(reload_args);
    execute(reloader, &args, &config_file)?;
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
        Ok(_) => "Ok".to_string(),
        Err(e) => e.to_string(),
    }
}
