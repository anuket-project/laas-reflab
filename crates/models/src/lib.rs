#![doc = include_str!("../README.md")]
#![feature(
    min_specialization,
    associated_type_defaults,
    never_type,
    negative_impls,
    trait_alias
)]

pub mod allocator;
pub mod dashboard;
pub mod inventory;

mod log;

pub use log::EasyLog;
