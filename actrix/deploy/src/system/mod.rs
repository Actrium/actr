//! System utilities module

mod dependencies;
mod firewall;
mod install;
mod releases;
mod service;
mod uninstall;

// Public exports
pub use dependencies::check_dependencies;
pub use install::{
    ServiceArgs, find_local_build_binary, install_from_source, install_systemd_service,
    rollback_command, status_command, update_service,
};
pub use uninstall::{UninstallArgs, uninstall_application};
