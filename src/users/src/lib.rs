// Copyright (c) 2023 University of New Hampshire
// SPDX-License-Identifier: MIT
#![doc = include_str!("../README.md")]
#![allow(dead_code, unused_variables)]

pub mod ipa;

use ipa::*;

use common::prelude::{anyhow, *};
use rand::Rng;

pub async fn create_vpn_user(username: String, group: String) -> Result<bool, anyhow::Error> {
    let mut ipa = IPA::init().await.expect("Expected to initialize ipa");
    match ipa.group_add_user(&group, &username).await {
        Ok(b) => return Ok(b),
        Err(e) => return Err(anyhow::Error::msg(e.to_string())),
    }
}

pub async fn reset_vpn_user(username: String) -> Result<bool, anyhow::Error> {
    let mut ipa = IPA::init().await.expect("Expected to initialize ipa");
    let pass: String = gen_rand_pass();
    match ipa
        .update_user(
            username,
            Vec::<UserData>::new(),
            vec![UserData::userpassword(Some(pass.clone()))],
            false,
        )
        .await
    {
        Ok(b) => {
            // notify user
            return Ok(b);
        }
        Err(e) => return Err(anyhow::Error::msg(e.to_string())),
    }
}

pub async fn delete_vpn_user(username: String, group: String) -> Result<bool, anyhow::Error> {
    let mut ipa = IPA::init().await.expect("Expected to initialize ipa");
    match ipa.group_remove_user(&group, &username).await {
        Ok(b) => return Ok(b),
        Err(e) => return Err(anyhow::Error::msg(e.to_string())),
    }
}

fn gen_rand_pass() -> String {
    let include_uppercase = true;
    let include_lowercase = true;
    let include_numbers = true;
    let special_chars = false;

    let min_length = 8;
    let max_length = 12;

    let mut rng = rand::thread_rng();
    let length: usize = rng.gen_range(min_length..max_length);

    let mut base = String::new();

    let mut sourcing_vec = vec![];

    if include_lowercase {
        sourcing_vec.append(&mut ('a'..='z').collect());
    }

    if include_uppercase {
        sourcing_vec.append(&mut ('A'..='Z').collect());
    }

    if include_numbers {
        sourcing_vec.append(&mut ('0'..='9').collect());
    }

    if special_chars {
        sourcing_vec.append(&mut "!@#$%^&*()-_=+;:/?.>,<~".chars().into_iter().collect());
    }

    for _ in 0..length {
        let ci = rng.gen_range(0..sourcing_vec.len());
        let c = sourcing_vec[ci];
        base.push(c);
    }

    base
}
