//! HTTP API DTOs for autonomicd.
//!
//! This crate is schema-only: it re-exports the canonical wire types from
//! `aios-protocol::homeostasis` so client crates (`life-kernel-facade`, etc.)
//! can depend on typed request/response shapes without pulling in autonomicd's
//! server runtime.

#![forbid(unsafe_code)]

pub use aios_protocol::homeostasis::{
    BudgetStateDto, EconomicMode, HomeostaticProjectionDto, HomeostaticStateDto, PillarStateDto,
};
