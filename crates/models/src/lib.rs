#![doc = include_str!("../README.md")]
#![feature(
    min_specialization,
    associated_type_defaults,
    never_type,
    generic_arg_infer,
    negative_impls,
    result_flattening,
    trait_alias
)]

pub mod allocator;
pub mod dashboard;
pub mod inventory;

mod log;

pub use log::EasyLog;
