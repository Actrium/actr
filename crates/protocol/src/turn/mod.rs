//! TURN username claims and token payloads.
//!
//! This module contains the lightweight, JSON-serializable structures that
//! travel inside TURN authentication usernames.

mod claims;

pub use claims::Claims;
