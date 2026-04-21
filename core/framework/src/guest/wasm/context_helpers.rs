//! Shared WIT ⇄ actr value-type translation helpers.
//!
//! Kept in a separate module from [`super::context`] so the adapter code
//! ([`super::adapter`]) can reuse them without having to depend on the
//! whole `Context`-trait impl surface. Every helper is a one-shot pure
//! conversion with no I/O or async side effects.

use actr_protocol::{ActrId, ActrType, Realm};

use crate::Dest;

use super::generated::actr::workload::types as wit_types;

pub fn realm_to_wit(r: &Realm) -> wit_types::Realm {
    wit_types::Realm { realm_id: r.realm_id }
}

pub fn realm_from_wit(r: &wit_types::Realm) -> Realm {
    Realm { realm_id: r.realm_id }
}

pub fn actr_type_to_wit(t: &ActrType) -> wit_types::ActrType {
    wit_types::ActrType {
        manufacturer: t.manufacturer.clone(),
        name: t.name.clone(),
        version: t.version.clone(),
    }
}

pub fn actr_type_from_wit(t: &wit_types::ActrType) -> ActrType {
    ActrType {
        manufacturer: t.manufacturer.clone(),
        name: t.name.clone(),
        version: t.version.clone(),
    }
}

pub fn actr_id_to_wit(id: &ActrId) -> wit_types::ActrId {
    wit_types::ActrId {
        realm: realm_to_wit(&id.realm),
        serial_number: id.serial_number,
        type_: actr_type_to_wit(&id.r#type),
    }
}

pub fn actr_id_from_wit(id: &wit_types::ActrId) -> ActrId {
    ActrId {
        realm: realm_from_wit(&id.realm),
        serial_number: id.serial_number,
        r#type: actr_type_from_wit(&id.type_),
    }
}

pub fn dest_to_wit(dest: &Dest) -> wit_types::Dest {
    match dest {
        Dest::Shell => wit_types::Dest::Shell,
        Dest::Local => wit_types::Dest::Local,
        Dest::Actor(id) => wit_types::Dest::Actor(actr_id_to_wit(id)),
    }
}
