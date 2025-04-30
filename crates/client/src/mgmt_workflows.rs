//! Management utility functions consumed by the CLI.
//! Includes functionality like booting booked hosts, setting host power states etc.

use dal::{new_client, AsEasyTransaction, DBTable, FKey};
use models::{
    allocator::ResourceHandle,
    inventory::{BootTo, Host, Lab},
};
use tascii::prelude::*;
use workflows::{
    deploy_booking::{
        set_boot::SetBoot,
        set_host_power_state::{get_host_power_state, HostConfig, PowerState, SetPower},
    },
    retry_for,
};

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct BootToDev {
    pub host: FKey<Host>,
    pub bootdev: BootTo,
}

tascii::mark_task!(BootToDev);
impl AsyncRunnable for BootToDev {
    type Output = ();

    async fn run(&mut self, context: &Context) -> Result<Self::Output, TaskError> {
        retry_for(SetPower::off(self.host), context, 5, 10)?;

        retry_for(
            SetBoot {
                host_id: self.host,
                persistent: true,
                boot_to: self.bootdev,
            },
            context,
            5,
            10,
        )?;

        retry_for(SetPower::on(self.host), context, 5, 10)?;

        Ok(())
    }

    fn summarize(&self, _id: dal::ID) -> String {
        todo!()
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("MGMTBootToDevTask").versioned(1)
    }
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct GetPowerState {
    host: FKey<Host>,
}

tascii::mark_task!(GetPowerState);
impl AsyncRunnable for GetPowerState {
    type Output = PowerState;

    async fn run(&mut self, _context: &Context) -> Result<Self::Output, TaskError> {
        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        let host = self.host.get(&mut transaction).await.unwrap();
        transaction.commit().await.unwrap();

        get_host_power_state(&HostConfig::try_from(host.into_inner()).unwrap())
            .await
            .map_err(|_| TaskError::Reason("Error getting power state".to_owned()))
    }

    fn summarize(&self, _id: dal::ID) -> String {
        todo!()
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("GetPowerState").versioned(1)
    }
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct BootBookedHosts {}

tascii::mark_task!(BootBookedHosts);
impl AsyncRunnable for BootBookedHosts {
    type Output = ();

    async fn run(&mut self, context: &Context) -> Result<Self::Output, TaskError> {
        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        for lab in match Lab::select().run(&mut transaction).await {
            Ok(v) => v,
            Err(e) => return Err(TaskError::Reason(format!("Unable to get labs: {e}"))),
        } {
            for (host, _handle) in ResourceHandle::query_allocated::<Host>(
                &mut transaction,
                lab.id,
                None,
                None,
                &[],
                &Vec::new(),
            )
            .await?
            {
                context.spawn(BootBookedHost { host });
            }
        }

        Ok(())
    }

    fn summarize(&self, _id: dal::ID) -> String {
        todo!()
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("BootBookedHosts").versioned(1)
    }
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
struct BootBookedHost {
    host: FKey<Host>,
}

tascii::mark_task!(BootBookedHost);
impl AsyncRunnable for BootBookedHost {
    type Output = ();

    async fn run(&mut self, context: &Context) -> Result<Self::Output, TaskError> {
        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        let host = self.host.get(&mut transaction).await.unwrap();
        transaction.commit().await.unwrap();

        let _ipmi_fqdn = &host.ipmi_fqdn;
        let _ipmi_admin_user = &host.ipmi_user;
        let _ipmi_admin_password = &host.ipmi_pass;

        let current_state = context.spawn(GetPowerState { host: self.host }).join()?;

        if let PowerState::Off = current_state {
            context
                .spawn(SetPower {
                    host: self.host,
                    pstate: PowerState::On,
                })
                .join()?;
        }

        Ok(())
    }

    fn summarize(&self, _id: dal::ID) -> String {
        todo!()
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("BootBookedHost").versioned(1)
    }
}
