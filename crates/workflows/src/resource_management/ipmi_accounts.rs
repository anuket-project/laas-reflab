use common::prelude::rand::{self, seq::SliceRandom, Rng};
use dal::{new_client, AsEasyTransaction, FKey, ID};
use models::inventory::Host;
use tascii::prelude::*;

use std::{process::Command, time::Duration};

use crate::deploy_booking::reachable::WaitReachable;

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct CreateIPMIAccount {
    pub host: FKey<Host>,
    pub password: String,
    pub username: String,
    pub userid: String,
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct DeleteIPMIAccount {
    pub host: FKey<Host>,
    pub userid: String,
}

tascii::mark_task!(DeleteIPMIAccount);
impl AsyncRunnable for DeleteIPMIAccount {
    type Output = ();

    async fn execute_task(&mut self, context: &Context) -> Result<Self::Output, TaskError> {
        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        let host = self.host.get(&mut transaction).await.unwrap();

        let ipmi_fqdn = &host.ipmi_fqdn;
        let ipmi_admin_user = &host.ipmi_user;
        let ipmi_admin_password = &host.ipmi_pass;

        let ipmi_url = context
            .spawn(WaitReachable {
                endpoint: ipmi_fqdn.clone(),
            })
            .join()?;

        // reset password to something "random"
        let _ipmi_cmd = Command::new("ipmitool")
            .args([
                "-I",
                "lanplus",
                "-C",
                "3",
                "-H",
                &ipmi_url,
                "-U",
                ipmi_admin_user,
                "-P",
                ipmi_admin_password,
                "user",
                "set",
                "password",
                &self.userid,
                &generate_password(15),
            ])
            .output()
            .expect("Failed to execute ipmitool command");

        let _ = transaction.commit().await;

        Ok(())
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("CreateIPMIAccount").versioned(1)
    }
    
    fn timeout() -> Duration {
        let estimated_overhead_time = Duration::from_secs(30);
        WaitReachable::overall_timeout() + estimated_overhead_time
    }
    
    fn retry_count() -> usize {
        0
    }
}

tascii::mark_task!(CreateIPMIAccount);
impl AsyncRunnable for CreateIPMIAccount {
    type Output = ();

    async fn execute_task(&mut self, context: &Context) -> Result<Self::Output, TaskError> {
        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        let host = self.host.get(&mut transaction).await.unwrap();

        let ipmi_fqdn = &host.ipmi_fqdn;
        let ipmi_admin_user = &host.ipmi_user;
        let ipmi_admin_password = &host.ipmi_pass;

        let ipmi_url = context
            .spawn(WaitReachable {
                endpoint: ipmi_fqdn.clone(),
            })
            .join()?;

        let _ipmi_cmd = Command::new("ipmitool")
            .args([
                "-I",
                "lanplus",
                "-C",
                "3",
                "-H",
                &ipmi_url,
                "-U",
                ipmi_admin_user,
                "-P",
                ipmi_admin_password,
                "user",
                "set",
                "password",
                &self.userid,
                &self.password,
            ])
            .output()
            .expect("Failed to execute ipmitool command");

        let _ipmi_cmd = Command::new("ipmitool")
            .args([
                "-I",
                "lanplus",
                "-C",
                "3",
                "-H",
                &ipmi_url,
                "-U",
                ipmi_admin_user,
                "-P",
                ipmi_admin_password,
                "user",
                "set",
                "name",
                &self.userid,
                &self.username,
            ])
            .output()
            .expect("Failed to execute ipmitool command");

        let _ = transaction.commit().await;

        Ok(())
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("CreateIPMIAccount").versioned(1)
    }
    
    fn timeout() -> Duration {
        let estimated_overhead_time = Duration::from_secs(30);
        WaitReachable::overall_timeout() + estimated_overhead_time
    }
    
    fn retry_count() -> usize {
        0
    }
}

pub fn generate_username(length: usize) -> String {
    let mut rng = rand::thread_rng();
    let mut s = String::with_capacity(length);

    let alphabet = Vec::from_iter('a'..='z');

    for _ in 0..length {
        let idx = rng.gen_range(0..alphabet.len());

        let c = alphabet[idx];

        s.push(c);
    }

    s
}

pub fn generate_password(length: usize) -> String {
    let mut rng = rand::thread_rng();

    let special_chars = ['#', '!', '@', '~'];

    let numbers = Vec::from_iter('0'..='9');

    let lowercase = Vec::from_iter('a'..='z');
    let uppercase = Vec::from_iter('A'..='Z');

    // this algorithm is not perfectly random, it
    // does have some additional sources of negentropy
    // in comparison to what is theoretically possible
    // *within the password requirements set by some hosts*,
    // but it tries to fuzz period boundaries and
    // does not repeat any character class pattern,
    // so it should still provide relatively good passwords
    // that are trivially (obviously) lower bounded at 4^length
    // and should be provably much higher (as average
    // character class length is higher than for special chars,
    // and since we shuffle within periods to provide
    // (length % 4) * 4! additional entropy)
    //
    // this fits the requirement that for a length 4 or greater,
    // all character classes are used at least once

    let inner_length = (length / 4) * 4 + 4; // div ceil

    let mut s = String::with_capacity(inner_length);

    for block in 0..(inner_length / 4) {
        let block_start = block * 4;
        let _block_end = block_start + 3;

        let mut classes = [
            &special_chars,
            numbers.as_slice(),
            lowercase.as_slice(),
            uppercase.as_slice(),
        ];

        // inefficient, but this is fine since this operation is rare
        classes.shuffle(&mut rng);

        for class in classes {
            let idx: usize = rng.gen_range(0..class.len());

            let c = class[idx];

            s.push(c);
        }
    }

    s[0..length].to_owned()
}
