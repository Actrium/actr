//! System utilities module

mod dependencies;
mod install;
mod uninstall;

// Public exports
pub use dependencies::check_dependencies;
pub use install::{install_application, install_systemd_service};
pub use uninstall::uninstall_application;
