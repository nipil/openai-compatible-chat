use std::ffi::OsString;
use std::path::PathBuf;

use service_manager::{
    RestartPolicy, ServiceInstallCtx, ServiceLabel, ServiceManager, ServiceStartCtx,
    ServiceStopCtx, ServiceUninstallCtx,
};
use thiserror::Error;
use whoami::username;

const SERVICE_LABEL: &str = "com.github.nipil.openai-compatible-cli-chat";

#[derive(Error, Debug)]
pub enum ServiceManagerError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Whoami error: {0}")]
    UserDetection(#[from] whoami::Error),

    #[error("Service manager error: {0}")]
    Manager(String),
}

fn current_exe_path() -> Result<PathBuf, ServiceManagerError> {
    std::env::current_exe().map_err(ServiceManagerError::Io)
}

fn get_label() -> Result<ServiceLabel, ServiceManagerError> {
    let label: ServiceLabel = SERVICE_LABEL
        .parse()
        .map_err(|e| ServiceManagerError::Manager(format!("Invalid label: {}", e)))?;
    Ok(label)
}

fn get_manager() -> Result<Box<dyn ServiceManager>, ServiceManagerError> {
    let manager = <dyn ServiceManager>::native().map_err(|e| {
        ServiceManagerError::Manager(format!("No service manager detected : {}", e))
    })?;
    Ok(manager)
}

/// Install the service as the SAME user that runs the install command
pub fn install(port: u16, bind_addr: &str) -> Result<(), ServiceManagerError> {
    let args = vec![
        OsString::from("--port"),
        OsString::from(port.to_string()),
        OsString::from("--bind-addr"),
        OsString::from(bind_addr),
    ];

    get_manager()?
        .install(ServiceInstallCtx {
            // https://docs.rs/service-manager/latest/service_manager/struct.ServiceInstallCtx.html#fields
            label: get_label()?,
            program: current_exe_path()?,
            args,
            contents: None,
            username: Some(username()?),
            working_directory: None,
            environment: None,
            autostart: true,
            restart_policy: RestartPolicy::OnFailure {
                delay_secs: Some(10),
                max_retries: None,
                reset_after_secs: Some(600),
            },
        })
        .map_err(|e| ServiceManagerError::Manager(e.to_string()))?;

    Ok(())
}

pub fn uninstall() -> Result<(), ServiceManagerError> {
    get_manager()?
        .uninstall(ServiceUninstallCtx {
            label: get_label()?,
        })
        .map_err(|e| ServiceManagerError::Manager(e.to_string()))?;

    Ok(())
}

pub fn start() -> Result<(), ServiceManagerError> {
    get_manager()?
        .start(ServiceStartCtx {
            label: get_label()?,
        })
        .map_err(|e| ServiceManagerError::Manager(e.to_string()))?;

    Ok(())
}

pub fn stop() -> Result<(), ServiceManagerError> {
    get_manager()?
        .stop(ServiceStopCtx {
            label: get_label()?,
        })
        .map_err(|e| ServiceManagerError::Manager(e.to_string()))?;

    Ok(())
}

pub fn restart() -> Result<(), ServiceManagerError> {
    stop()?;
    start()?;
    Ok(())
}
