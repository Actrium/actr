//! TURN username claims and token payloads.
//!
//! This module contains the lightweight, JSON-serializable structures that
//! travel inside TURN authentication usernames.

mod claims;
mod token;

pub use claims::Claims;
pub use token::Token;
