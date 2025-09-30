//! Management utility functions consumed by the CLI.
//! Includes functionality like booting booked hosts, setting host power states etc.

use std::time::Duration;

use dal::{new_client, AsEasyTransaction, DBTable, FKey};
use models::{
    allocator::ResourceHandle,
    inventory::{BootTo, Host, Lab},
};
use tascii::prelude::*;
use workflows::deploy_booking::{
    set_boot::SetBoot,
    set_host_power_state::{get_host_power_state, HostConfig, PowerState, SetPower},
};

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct BootToDev {
    pub host: FKey<Host>,
    pub bootdev: BootTo,
}

tascii::mark_task!(BootToDev);
impl AsyncRunnable for BootToDev {
    type Output = ();

    async fn execute_task(&mut self, context: &Context) -> Result<Self::Output, TaskError> {
        context.spawn(SetPower::off(self.host)).join()?;
        context
            .spawn(SetBoot {
                host_id: self.host,
                persistent: true,
                boot_to: self.bootdev,
            })
            .join()?;

        context.spawn(SetPower::on(self.host)).join()?;

        Ok(())
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("MGMTBootToDevTask").versioned(1)
    }

    fn timeout() -> std::time::Duration {
        let estimated_overhead_time = Duration::from_secs(30);
        SetPower::overall_timeout() + SetBoot::overall_timeout() + estimated_overhead_time
    }

    fn retry_count() -> usize {
        1
    }
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct GetPowerState {
    host: FKey<Host>,
}

tascii::mark_task!(GetPowerState);
impl AsyncRunnable for GetPowerState {
    type Output = PowerState;

    async fn execute_task(&mut self, _context: &Context) -> Result<Self::Output, TaskError> {
        let mut client = new_client().await.unwrap();
        let mut transaction = client.easy_transaction().await.unwrap();

        let host = self.host.get(&mut transaction).await.unwrap();
        transaction.commit().await.unwrap();

        get_host_power_state(&HostConfig::try_from(host.into_inner()).unwrap())
            .await
            .map_err(|_| TaskError::Reason("Error getting power state".to_owned()))
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("GetPowerState").versioned(1)
    }

    fn timeout() -> Duration {
        Duration::from_secs(60)
    }

    fn retry_count() -> usize {
        20
    }
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct BootBookedHosts {}

tascii::mark_task!(BootBookedHosts);
impl AsyncRunnable for BootBookedHosts {
    type Output = ();

    async fn execute_task(&mut self, context: &Context) -> Result<Self::Output, TaskError> {
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
                context.spawn(SetPower::on(host));
            }
        }

        Ok(())
    }

    fn identifier() -> TaskIdentifier {
        TaskIdentifier::named("BootBookedHosts").versioned(1)
    }

    fn timeout() -> Duration {
        // Does not wait for all hosts to boot to return task result
        Duration::from_secs(5 * 60)
    }

    fn retry_count() -> usize {
        0
    }
}
