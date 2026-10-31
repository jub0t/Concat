#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

// bindgen's bitfield accessors transmute between integer sizes that are
// plain casts; clippy lints each one, and the workspace's CI denies
// warnings. The allowance for that lives in Cargo.toml's [lints] table:
// a lint on macro-expanded code takes its level from the compiler's
// command line, which is where that table lands it, and not from any
// attribute this file can carry.
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
