//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use models::{abstractions::*, allocation::*, dashboard::*, inventory::*};

pub fn mark_host_not_working(host: String, reason: String) {}

pub fn image_host_for_servicing(host: String, reason: String, image: Option<String>) {
    let mut client = new_client().expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .expect("Transaction creation error");

    let agg_id: FKey<Aggregate>;
    let host_profile = Host::get_by_name(&mut transaction, host);

    let image = match image {
        Some(_) => image.unwrap(),
        None => todo!(),
    };
    let instance = Instance {
        id: todo!(),
        within_template: todo!(),
        config: todo!(),
        network_data: todo!(),
        logs: todo!(),
    };

    let agg = Aggregate {
        id: FKey::new_id_dangling(),
        deleted: false,
        instances: todo!(),
        users: todo!(),
        vlans: todo!(),
    };
}

pub fn select_image() -> FKey<Image> {
    let mut client = new_client().expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .expect("Transaction creation error");

    let images = Image::select()
        .where_field("deleted")
        .equals(true)
        .where_field("name")
        .like("Ubuntu%")
        .run(&mut transaction)
        .unwrap();

    todo!()
}

pub fn get_aggregates() {
    let mut base = Aggregate::select();
    let mut client = new_client().expect("Expected to connect to db");
    let mut transaction = client
        .easy_transaction()
        .expect("Transaction creation error");

    // in a loop, get things people want to select on
    loop {
        // get field
        let field_name = todo!();
        let wb = base.where_field(field_name);

        // get operation they want to do, execute something like this
        let value = "";
        base = wb.equals(value);
    }

    // after loop, run the query
    let aggs = base.run(&mut transaction).expect("couldn't run thing");
}

