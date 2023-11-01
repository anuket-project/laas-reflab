//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

#![allow(dead_code, unused_variables, unused_imports, incomplete_features)]
#![feature(
    min_specialization,
    async_fn_in_trait,
    associated_type_defaults,
    generic_const_exprs,
    never_type,
    generic_arg_infer,
    negative_impls,
    result_flattening,
    trait_alias,
)]

use common::prelude::*;
use tokio_postgres::Client;

//pub mod resources;
pub mod allocation;
pub mod dashboard;
pub mod inventory;

pub mod dal {
    pub use dal::*;
}

pub mod postgres {
    pub use tokio_postgres::*;
}

pub mod macaddr {
    pub use eui48::*;
}

mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}
