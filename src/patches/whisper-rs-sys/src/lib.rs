#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
// bindgen's bitfield accessors transmute between integer sizes that are
// plain casts; clippy lints each one, and the workspace's CI denies
// warnings, so the generated code is allowed it here alongside the naming
// lints above. The clippy:: form is inert under plain rustc.
#![allow(clippy::unnecessary_transmutes)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
